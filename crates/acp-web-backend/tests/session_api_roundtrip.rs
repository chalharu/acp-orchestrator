use std::{future::pending, pin::Pin, time::Duration};

use acp_app_support::{wait_for_health, wait_for_tcp_connect};
use acp_contracts::{
    CancelTurnResponse, CreateSessionResponse, MessageRole, PermissionDecision, PromptRequest,
    ResolvePermissionRequest, ResolvePermissionResponse, StreamEvent, StreamEventPayload,
};
use acp_mock::{MockConfig, spawn_with_shutdown_task};
use acp_web_backend::{AppState, ServerConfig, serve_with_shutdown as serve_backend_with_shutdown};
use anyhow::{Context, Result};
use eventsource_stream::Eventsource;
use futures_util::{Stream, StreamExt};
use reqwest::{Client, StatusCode};
use tokio::{net::TcpListener, sync::oneshot, task::JoinHandle, time::sleep};

type SseStream = Pin<Box<dyn Stream<Item = Result<StreamEvent>> + Send>>;

#[tokio::test]
async fn prompt_submission_streams_snapshot_user_and_assistant_messages() -> Result<()> {
    let stack = TestStack::spawn(ServerConfig {
        session_cap: 8,
        acp_server: String::new(),
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
        acp_server: String::new(),
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
        acp_server: String::new(),
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
        acp_server: String::new(),
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

#[tokio::test]
async fn session_history_returns_messages_after_a_roundtrip() -> Result<()> {
    let stack = TestStack::spawn(ServerConfig {
        session_cap: 8,
        acp_server: String::new(),
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
        .submit_prompt("alice", &session.session.id, "history please")
        .await?;
    let _ = expect_next_event(&mut events).await?;
    let _ = expect_next_event(&mut events).await?;

    let history = stack.session_history("alice", &session.session.id).await?;
    assert_eq!(history.messages.len(), 2);
    assert!(matches!(history.messages[0].role, MessageRole::User));
    assert_eq!(history.messages[0].text, "history please");
    assert!(matches!(history.messages[1].role, MessageRole::Assistant));

    Ok(())
}

#[tokio::test]
async fn prompt_submission_streams_mock_failures_as_status_messages() -> Result<()> {
    let stack = TestStack::spawn(ServerConfig {
        session_cap: 8,
        acp_server: "127.0.0.1:9".to_string(),
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
        .submit_prompt("alice", &session.session.id, "this will fail")
        .await?;

    let user_message = expect_next_event(&mut events).await?;
    assert!(matches!(
        user_message.payload,
        StreamEventPayload::ConversationMessage { message }
            if matches!(message.role, MessageRole::User)
    ));

    let status = expect_next_event(&mut events).await?;
    assert!(matches!(
        status.payload,
        StreamEventPayload::Status { message } if message.starts_with("ACP request failed:")
    ));

    Ok(())
}

#[tokio::test]
async fn permission_requests_can_be_approved_through_http() -> Result<()> {
    let stack = TestStack::spawn(ServerConfig {
        session_cap: 8,
        acp_server: String::new(),
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
        .submit_prompt("alice", &session.session.id, "permission please")
        .await?;

    let user_message = expect_next_event(&mut events).await?;
    assert!(matches!(
        user_message.payload,
        StreamEventPayload::ConversationMessage { message }
            if matches!(message.role, MessageRole::User)
    ));

    let permission = expect_next_event(&mut events).await?;
    match permission.payload {
        StreamEventPayload::PermissionRequested { request } => {
            assert_eq!(request.request_id, "req_1");
            assert_eq!(request.summary, "read_text_file README.md");
        }
        payload => panic!("unexpected payload: {payload:?}"),
    }

    let resolution = stack
        .resolve_permission(
            "alice",
            &session.session.id,
            "req_1",
            PermissionDecision::Approve,
        )
        .await?;
    assert_eq!(resolution.request_id, "req_1");

    let assistant_message = expect_next_event(&mut events).await?;
    assert!(matches!(
        assistant_message.payload,
        StreamEventPayload::ConversationMessage { message }
            if matches!(message.role, MessageRole::Assistant)
                && message.text.starts_with("mock assistant:")
    ));

    Ok(())
}

#[tokio::test]
async fn permission_requests_can_be_denied_without_recording_an_assistant_reply() -> Result<()> {
    let stack = TestStack::spawn(ServerConfig {
        session_cap: 8,
        acp_server: String::new(),
    })
    .await?;
    let session = stack.create_session("alice").await?;
    let mut events = stack.open_events("alice", &session.session.id).await?;

    let _ = expect_next_event(&mut events).await?;
    stack
        .submit_prompt("alice", &session.session.id, "permission please")
        .await?;
    let _ = expect_next_event(&mut events).await?;
    let permission = expect_next_event(&mut events).await?;
    assert!(matches!(
        permission.payload,
        StreamEventPayload::PermissionRequested { .. }
    ));

    let resolution = stack
        .resolve_permission(
            "alice",
            &session.session.id,
            "req_1",
            PermissionDecision::Deny,
        )
        .await?;
    assert_eq!(resolution.request_id, "req_1");
    tokio::time::sleep(Duration::from_millis(100)).await;

    let history = stack.session_history("alice", &session.session.id).await?;
    assert_eq!(history.messages.len(), 1);
    assert_eq!(history.messages[0].text, "permission please");

    Ok(())
}

#[tokio::test]
async fn cancelling_a_pending_permission_turn_returns_a_status_event() -> Result<()> {
    let stack = TestStack::spawn(ServerConfig {
        session_cap: 8,
        acp_server: String::new(),
    })
    .await?;
    let session = stack.create_session("alice").await?;
    let mut events = stack.open_events("alice", &session.session.id).await?;

    let _ = expect_next_event(&mut events).await?;
    stack
        .submit_prompt("alice", &session.session.id, "permission please")
        .await?;
    let _ = expect_next_event(&mut events).await?;
    let permission = expect_next_event(&mut events).await?;
    assert!(matches!(
        permission.payload,
        StreamEventPayload::PermissionRequested { .. }
    ));

    let cancelled = stack.cancel_turn("alice", &session.session.id).await?;
    assert!(cancelled.cancelled);

    let status = expect_next_event(&mut events).await?;
    assert!(matches!(
        status.payload,
        StreamEventPayload::Status { message } if message == "turn cancelled"
    ));

    Ok(())
}

#[tokio::test]
async fn resolving_unknown_permission_requests_returns_not_found() -> Result<()> {
    let stack = TestStack::spawn(ServerConfig {
        session_cap: 8,
        acp_server: String::new(),
    })
    .await?;
    let session = stack.create_session("alice").await?;

    let response = stack
        .client
        .post(format!(
            "{}/api/v1/sessions/{}/permissions/req_missing",
            stack.backend_url, session.session.id
        ))
        .bearer_auth("alice")
        .json(&ResolvePermissionRequest {
            decision: PermissionDecision::Approve,
        })
        .send()
        .await
        .context("resolving a missing permission request")?;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    Ok(())
}

#[tokio::test]
async fn lagged_event_streams_continue_after_dropping_backlog() -> Result<()> {
    let stack = TestStack::spawn(ServerConfig {
        session_cap: 8,
        acp_server: "127.0.0.1:9".to_string(),
    })
    .await?;
    let session = stack.create_session("alice").await?;
    let mut events = stack.open_events("alice", &session.session.id).await?;

    for index in 0..80 {
        stack
            .submit_prompt("alice", &session.session.id, &format!("prompt {index}"))
            .await?;
    }
    sleep(Duration::from_millis(200)).await;

    let snapshot = expect_next_event(&mut events).await?;
    assert!(matches!(
        snapshot.payload,
        StreamEventPayload::SessionSnapshot { .. }
    ));

    let resumed = expect_next_event(&mut events).await?;
    assert!(matches!(
        resumed.payload,
        StreamEventPayload::ConversationMessage { .. } | StreamEventPayload::Status { .. }
    ));

    Ok(())
}

#[tokio::test]
async fn direct_mock_server_accepts_tcp_connections() -> Result<()> {
    let (address, shutdown, handle) = spawn_direct_mock_server().await?;

    wait_for_stack_tcp(&address).await?;

    let _ = shutdown.send(());
    handle
        .await
        .context("joining direct mock task")?
        .context("mock server should stop cleanly")?;
    Ok(())
}

#[tokio::test]
async fn direct_backend_server_reports_health() -> Result<()> {
    let client = Client::builder().build().context("building test client")?;
    let (base_url, handle) = spawn_direct_backend_server("127.0.0.1:9".to_string()).await?;

    wait_for_stack_health(&client, &base_url).await?;

    handle.abort();
    let _ = handle.await;
    Ok(())
}

#[tokio::test]
async fn graceful_mock_server_shutdown_completes_cleanly() -> Result<()> {
    let (address, shutdown, handle) = spawn_graceful_mock_server().await?;

    wait_for_stack_tcp(&address).await?;
    let _ = shutdown.send(());
    handle
        .await
        .context("joining graceful mock task")?
        .context("mock server should stop cleanly")?;

    Ok(())
}

#[tokio::test]
async fn graceful_backend_server_shutdown_completes_cleanly() -> Result<()> {
    let client = Client::builder().build().context("building test client")?;
    let (base_url, shutdown, handle) =
        spawn_graceful_backend_server("127.0.0.1:9".to_string()).await?;

    wait_for_stack_health(&client, &base_url).await?;
    let _ = shutdown.send(());
    handle
        .await
        .context("joining graceful backend task")?
        .context("backend server should stop cleanly")?;

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
        let client = Client::builder().build().context("building test client")?;
        let mut mock_shutdown = None;
        if backend_config.acp_server.is_empty() {
            let mock_listener = TcpListener::bind("127.0.0.1:0")
                .await
                .context("binding mock listener")?;
            let mock_address = mock_listener.local_addr().context("reading mock address")?;
            let (mock_shutdown_tx, mock_shutdown_rx) = oneshot::channel();
            spawn_with_shutdown_task(mock_listener, MockConfig::default(), async move {
                let _ = mock_shutdown_rx.await;
            });

            backend_config.acp_server = mock_address.to_string();
            wait_for_stack_tcp(&backend_config.acp_server).await?;
            mock_shutdown = Some(mock_shutdown_tx);
        }

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
            if let Err(error) = serve_backend_with_shutdown(backend_listener, state, shutdown).await
            {
                eprintln!("test backend stopped: {error}");
            }
        });

        let backend_url = format!("http://{backend_address}");
        wait_for_stack_health(&client, &backend_url).await?;

        Ok(Self {
            backend_url,
            client,
            backend_shutdown: Some(backend_shutdown_tx),
            mock_shutdown,
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

    async fn cancel_turn(&self, token: &str, session_id: &str) -> Result<CancelTurnResponse> {
        let response = self
            .client
            .post(format!(
                "{}/api/v1/sessions/{session_id}/cancel",
                self.backend_url
            ))
            .bearer_auth(token)
            .send()
            .await
            .context("cancelling the test turn")?
            .error_for_status()
            .context("cancel turn returned an error")?;
        response.json().await.context("decoding cancel response")
    }

    async fn resolve_permission(
        &self,
        token: &str,
        session_id: &str,
        request_id: &str,
        decision: PermissionDecision,
    ) -> Result<ResolvePermissionResponse> {
        let response = self
            .client
            .post(format!(
                "{}/api/v1/sessions/{session_id}/permissions/{request_id}",
                self.backend_url
            ))
            .bearer_auth(token)
            .json(&ResolvePermissionRequest { decision })
            .send()
            .await
            .context("resolving test permission")?
            .error_for_status()
            .context("permission resolution returned an error")?;
        response
            .json()
            .await
            .context("decoding permission resolution response")
    }

    async fn session_history(
        &self,
        token: &str,
        session_id: &str,
    ) -> Result<acp_contracts::SessionHistoryResponse> {
        let response = self
            .client
            .get(format!(
                "{}/api/v1/sessions/{session_id}/history",
                self.backend_url
            ))
            .bearer_auth(token)
            .send()
            .await
            .context("loading test history")?
            .error_for_status()
            .context("history request returned an error")?;
        response.json().await.context("decoding history response")
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

async fn wait_for_stack_health(client: &Client, base_url: &str) -> Result<()> {
    wait_for_health(client, base_url, 50, Duration::from_millis(50))
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))
}

async fn wait_for_stack_tcp(address: &str) -> Result<()> {
    wait_for_tcp_connect(address, 50, Duration::from_millis(50))
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))
}

async fn spawn_direct_mock_server()
-> Result<(String, oneshot::Sender<()>, JoinHandle<std::io::Result<()>>)> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .context("binding direct mock listener")?;
    let address = listener
        .local_addr()
        .context("reading direct mock address")?;
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let handle = spawn_with_shutdown_task(listener, MockConfig::default(), async move {
        let _ = shutdown_rx.await;
    });

    Ok((address.to_string(), shutdown_tx, handle))
}

async fn spawn_direct_backend_server(
    mock_address: String,
) -> Result<(String, JoinHandle<std::io::Result<()>>)> {
    let state = AppState::new(ServerConfig {
        session_cap: 8,
        acp_server: mock_address,
    })
    .context("building direct backend state")?;
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .context("binding direct backend listener")?;
    let address = listener
        .local_addr()
        .context("reading direct backend address")?;
    let handle =
        tokio::spawn(async move { serve_backend_with_shutdown(listener, state, pending()).await });

    Ok((format!("http://{address}"), handle))
}

async fn spawn_graceful_mock_server()
-> Result<(String, oneshot::Sender<()>, JoinHandle<std::io::Result<()>>)> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .context("binding graceful mock listener")?;
    let address = listener
        .local_addr()
        .context("reading graceful mock address")?;
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let handle = spawn_with_shutdown_task(listener, MockConfig::default(), async move {
        let _ = shutdown_rx.await;
    });

    Ok((address.to_string(), shutdown_tx, handle))
}

async fn spawn_graceful_backend_server(
    mock_address: String,
) -> Result<(String, oneshot::Sender<()>, JoinHandle<std::io::Result<()>>)> {
    let state = AppState::new(ServerConfig {
        session_cap: 8,
        acp_server: mock_address,
    })
    .context("building graceful backend state")?;
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .context("binding graceful backend listener")?;
    let address = listener
        .local_addr()
        .context("reading graceful backend address")?;
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let handle = tokio::spawn(async move {
        let shutdown = async move {
            let _ = shutdown_rx.await;
        };
        serve_backend_with_shutdown(listener, state, shutdown).await
    });

    Ok((format!("http://{address}"), shutdown_tx, handle))
}
