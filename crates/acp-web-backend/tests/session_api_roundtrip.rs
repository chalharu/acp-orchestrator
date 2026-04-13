use std::{pin::Pin, time::Duration};

use acp_contracts::{
    CreateSessionResponse, MessageRole, PromptRequest, StreamEvent, StreamEventPayload,
};
use acp_mock::{MockConfig, serve_with_shutdown as serve_mock_with_shutdown};
use acp_web_backend::{AppState, ServerConfig, serve_with_shutdown};
use anyhow::{Context, Result};
use eventsource_stream::Eventsource;
use futures_util::{Stream, StreamExt};
use reqwest::{Client, StatusCode};
use tokio::{net::TcpListener, sync::oneshot, time::sleep};

type SseStream = Pin<Box<dyn Stream<Item = Result<StreamEvent>> + Send>>;

#[tokio::test]
async fn prompt_submission_streams_snapshot_user_and_assistant_messages() -> Result<()> {
    let stack = TestStack::spawn(ServerConfig {
        session_cap: 8,
        mock_url: String::new(),
    })
    .await?;

    let session = stack.create_session("alice").await?;
    let mut events = stack.open_events("alice", &session.session.id).await?;

    let snapshot = expect_next_event(&mut events).await?;
    assert!(matches!(
        snapshot.payload,
        StreamEventPayload::SessionSnapshot { .. }
    ));

    stack
        .submit_prompt("alice", &session.session.id, "hello through backend")
        .await?;

    let user_message = expect_next_event(&mut events).await?;
    match user_message.payload {
        StreamEventPayload::ConversationMessage { message } => {
            assert_eq!(message.text, "hello through backend");
            assert!(matches!(message.role, MessageRole::User));
        }
        payload => panic!("unexpected payload: {payload:?}"),
    }

    let assistant_message = expect_next_event(&mut events).await?;
    match assistant_message.payload {
        StreamEventPayload::ConversationMessage { message } => {
            assert!(matches!(message.role, MessageRole::Assistant));
            assert!(message.text.starts_with("mock assistant:"));
        }
        payload => panic!("unexpected payload: {payload:?}"),
    }

    Ok(())
}

#[tokio::test]
async fn session_lookup_rejects_different_principal() -> Result<()> {
    let stack = TestStack::spawn(ServerConfig {
        session_cap: 8,
        mock_url: String::new(),
    })
    .await?;
    let session = stack.create_session("alice").await?;

    let response = stack
        .client
        .get(format!(
            "{}/api/v1/sessions/{}",
            stack.backend_url, session.session.id
        ))
        .bearer_auth("bob")
        .send()
        .await
        .context("requesting session as the wrong principal")?;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    Ok(())
}

#[tokio::test]
async fn session_creation_enforces_principal_session_cap() -> Result<()> {
    let stack = TestStack::spawn(ServerConfig {
        session_cap: 1,
        mock_url: String::new(),
    })
    .await?;

    let first = stack.create_session("alice").await?;
    assert!(first.session.id.starts_with("s_"));

    let response = stack
        .client
        .post(format!("{}/api/v1/sessions", stack.backend_url))
        .bearer_auth("alice")
        .send()
        .await
        .context("creating a second session for alice")?;

    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    Ok(())
}

#[tokio::test]
async fn retention_prunes_oldest_closed_sessions() -> Result<()> {
    let stack = TestStack::spawn(ServerConfig {
        session_cap: 128,
        mock_url: String::new(),
    })
    .await?;

    let mut first_session_id = None;
    let mut last_session_id = None;

    for index in 0..33 {
        let created = stack.create_session("alice").await?;
        if index == 0 {
            first_session_id = Some(created.session.id.clone());
        }
        last_session_id = Some(created.session.id.clone());
        stack.close_session("alice", &created.session.id).await?;
    }

    let first_session_response = stack
        .client
        .get(format!(
            "{}/api/v1/sessions/{}",
            stack.backend_url,
            first_session_id.expect("first session id should exist")
        ))
        .bearer_auth("alice")
        .send()
        .await
        .context("loading the oldest closed session")?;
    assert_eq!(first_session_response.status(), StatusCode::NOT_FOUND);

    let last_session_response = stack
        .client
        .get(format!(
            "{}/api/v1/sessions/{}",
            stack.backend_url,
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
    next.context("SSE stream ended unexpectedly")?
}

struct TestStack {
    backend_url: String,
    client: Client,
    backend_shutdown: Option<oneshot::Sender<()>>,
    mock_shutdown: Option<oneshot::Sender<()>>,
}

impl TestStack {
    async fn spawn(mut backend_config: ServerConfig) -> Result<Self> {
        let mock_listener = TcpListener::bind("127.0.0.1:0")
            .await
            .context("binding mock listener")?;
        let mock_address = mock_listener.local_addr().context("reading mock address")?;
        let (mock_shutdown_tx, mock_shutdown_rx) = oneshot::channel();
        tokio::spawn(async move {
            let shutdown = async move {
                let _ = mock_shutdown_rx.await;
            };
            if let Err(error) =
                serve_mock_with_shutdown(mock_listener, MockConfig::default(), shutdown).await
            {
                eprintln!("test mock stopped: {error}");
            }
        });

        let mock_url = format!("http://{mock_address}");
        let client = Client::builder().build().context("building test client")?;
        wait_for_health(&client, &mock_url).await?;

        backend_config.mock_url = mock_url;
        let state = AppState::new(backend_config).context("building backend state")?;
        let backend_listener = TcpListener::bind("127.0.0.1:0")
            .await
            .context("binding backend listener")?;
        let backend_address = backend_listener
            .local_addr()
            .context("reading backend address")?;
        let (backend_shutdown_tx, backend_shutdown_rx) = oneshot::channel();
        tokio::spawn(async move {
            let shutdown = async move {
                let _ = backend_shutdown_rx.await;
            };
            if let Err(error) = serve_with_shutdown(backend_listener, state, shutdown).await {
                eprintln!("test backend stopped: {error}");
            }
        });

        let backend_url = format!("http://{backend_address}");
        wait_for_health(&client, &backend_url).await?;

        Ok(Self {
            backend_url,
            client,
            backend_shutdown: Some(backend_shutdown_tx),
            mock_shutdown: Some(mock_shutdown_tx),
        })
    }

    async fn create_session(&self, token: &str) -> Result<CreateSessionResponse> {
        let response = self
            .client
            .post(format!("{}/api/v1/sessions", self.backend_url))
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
                self.backend_url
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
                self.backend_url
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
                self.backend_url
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

impl Drop for TestStack {
    fn drop(&mut self) {
        if let Some(shutdown) = self.backend_shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(shutdown) = self.mock_shutdown.take() {
            let _ = shutdown.send(());
        }
    }
}

async fn wait_for_health(client: &Client, base_url: &str) -> Result<()> {
    let health_url = format!("{base_url}/healthz");

    for _ in 0..50 {
        if let Ok(response) = client.get(&health_url).send().await
            && response.status().is_success()
        {
            return Ok(());
        }
        sleep(Duration::from_millis(50)).await;
    }

    Err(anyhow::anyhow!(
        "health check did not succeed for {health_url}"
    ))
}
