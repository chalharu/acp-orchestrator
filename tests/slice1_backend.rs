use std::{pin::Pin, time::Duration};

use acp_orchestrator::{
    AppState, ServerConfig, app,
    models::{CreateSessionResponse, PromptRequest, StreamEvent, StreamEventPayload},
};
use anyhow::{Context, Result};
use eventsource_stream::Eventsource;
use futures_util::{Stream, StreamExt};
use reqwest::{Client, StatusCode};
use tokio::{net::TcpListener, sync::oneshot};

type SseStream = Pin<Box<dyn Stream<Item = Result<StreamEvent>> + Send>>;

#[tokio::test]
async fn prompt_round_trip_streams_snapshot_user_and_assistant_events() -> Result<()> {
    let server = TestServer::spawn(ServerConfig {
        session_cap: 8,
        assistant_delay: Duration::from_millis(5),
    })
    .await?;

    let session = server.create_session("alice").await?;
    let mut events = server.open_events("alice", &session.session.id).await?;

    let snapshot = expect_next_event(&mut events).await?;
    assert!(matches!(
        snapshot.payload,
        StreamEventPayload::SessionSnapshot { .. }
    ));

    server
        .submit_prompt("alice", &session.session.id, "hello from slice1")
        .await?;

    let user_message = expect_next_event(&mut events).await?;
    match user_message.payload {
        StreamEventPayload::ConversationMessage { message } => {
            assert_eq!(message.text, "hello from slice1");
            assert!(matches!(message.role, acp_orchestrator::MessageRole::User));
        }
        payload => panic!("unexpected payload: {payload:?}"),
    }

    let assistant_message = expect_next_event(&mut events).await?;
    match assistant_message.payload {
        StreamEventPayload::ConversationMessage { message } => {
            assert!(matches!(
                message.role,
                acp_orchestrator::MessageRole::Assistant
            ));
            assert!(message.text.starts_with("slice1 mock assistant:"));
        }
        payload => panic!("unexpected payload: {payload:?}"),
    }

    Ok(())
}

#[tokio::test]
async fn owner_check_rejects_other_principals() -> Result<()> {
    let server = TestServer::spawn(ServerConfig::default()).await?;
    let session = server.create_session("alice").await?;

    let response = server
        .client
        .get(format!(
            "{}/api/v1/sessions/{}",
            server.base_url, session.session.id
        ))
        .bearer_auth("bob")
        .send()
        .await
        .context("requesting session as the wrong principal")?;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    Ok(())
}

#[tokio::test]
async fn session_cap_is_enforced_per_principal() -> Result<()> {
    let server = TestServer::spawn(ServerConfig {
        session_cap: 1,
        assistant_delay: Duration::from_millis(5),
    })
    .await?;

    let first = server.create_session("alice").await?;
    assert!(first.session.id.starts_with("s_"));

    let response = server
        .client
        .post(format!("{}/api/v1/sessions", server.base_url))
        .bearer_auth("alice")
        .send()
        .await
        .context("creating a second session for alice")?;

    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    Ok(())
}

#[tokio::test]
async fn closed_sessions_are_pruned_after_the_retention_limit() -> Result<()> {
    let server = TestServer::spawn(ServerConfig {
        session_cap: 128,
        assistant_delay: Duration::from_millis(5),
    })
    .await?;

    let mut first_session_id = None;
    let mut last_session_id = None;

    for index in 0..33 {
        let created = server.create_session("alice").await?;
        if index == 0 {
            first_session_id = Some(created.session.id.clone());
        }
        last_session_id = Some(created.session.id.clone());
        server.close_session("alice", &created.session.id).await?;
    }

    let first_session_response = server
        .client
        .get(format!(
            "{}/api/v1/sessions/{}",
            server.base_url,
            first_session_id.expect("first session id should exist")
        ))
        .bearer_auth("alice")
        .send()
        .await
        .context("loading the oldest closed session")?;
    assert_eq!(first_session_response.status(), StatusCode::NOT_FOUND);

    let last_session_response = server
        .client
        .get(format!(
            "{}/api/v1/sessions/{}",
            server.base_url,
            last_session_id.expect("last session id should exist")
        ))
        .bearer_auth("alice")
        .send()
        .await
        .context("loading the newest closed session")?;
    assert_eq!(last_session_response.status(), StatusCode::OK);

    Ok(())
}

async fn expect_next_event(stream: &mut SseStream) -> Result<StreamEvent> {
    let next = tokio::time::timeout(Duration::from_secs(2), stream.next())
        .await
        .context("timed out waiting for SSE event")?;
    let next = next.context("SSE stream ended unexpectedly")?;
    next
}

struct TestServer {
    base_url: String,
    client: Client,
    shutdown: Option<oneshot::Sender<()>>,
}

impl TestServer {
    async fn spawn(config: ServerConfig) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .context("binding test listener")?;
        let address = listener.local_addr().context("reading test address")?;
        let (shutdown_tx, shutdown_rx) = oneshot::channel();

        tokio::spawn(async move {
            let app = app(AppState::new(config));
            let shutdown = async move {
                let _ = shutdown_rx.await;
            };
            if let Err(error) = axum::serve(listener, app)
                .with_graceful_shutdown(shutdown)
                .await
            {
                eprintln!("test server stopped: {error}");
            }
        });

        Ok(Self {
            base_url: format!("http://{address}"),
            client: Client::builder().build().context("building test client")?,
            shutdown: Some(shutdown_tx),
        })
    }

    async fn create_session(&self, token: &str) -> Result<CreateSessionResponse> {
        let response = self
            .client
            .post(format!("{}/api/v1/sessions", self.base_url))
            .bearer_auth(token)
            .send()
            .await
            .context("creating test session")?
            .error_for_status()
            .context("session creation returned an error")?;
        response.json().await.context("decoding session response")
    }

    async fn submit_prompt(&self, token: &str, session_id: &str, prompt: &str) -> Result<()> {
        self.client
            .post(format!(
                "{}/api/v1/sessions/{session_id}/messages",
                self.base_url
            ))
            .bearer_auth(token)
            .json(&PromptRequest {
                text: prompt.to_string(),
            })
            .send()
            .await
            .context("submitting test prompt")?
            .error_for_status()
            .context("prompt submission returned an error")?;
        Ok(())
    }

    async fn close_session(&self, token: &str, session_id: &str) -> Result<()> {
        self.client
            .post(format!(
                "{}/api/v1/sessions/{session_id}/close",
                self.base_url
            ))
            .bearer_auth(token)
            .send()
            .await
            .context("closing test session")?
            .error_for_status()
            .context("close session returned an error")?;
        Ok(())
    }

    async fn open_events(&self, token: &str, session_id: &str) -> Result<SseStream> {
        let response = self
            .client
            .get(format!(
                "{}/api/v1/sessions/{session_id}/events",
                self.base_url
            ))
            .bearer_auth(token)
            .send()
            .await
            .context("opening event stream")?
            .error_for_status()
            .context("event stream returned an error")?;

        let stream = response.bytes_stream().eventsource().map(|event| {
            let event = event.context("reading test event")?;
            serde_json::from_str(&event.data).context("decoding test event payload")
        });

        Ok(Box::pin(stream))
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
    }
}
