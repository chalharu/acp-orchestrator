use std::{future::pending, pin::Pin, time::Duration};

use acp_app_support::{wait_for_health, wait_for_tcp_connect};
use acp_contracts::{
    CancelTurnResponse, CreateSessionResponse, PromptRequest, ResolvePermissionRequest,
    ResolvePermissionResponse,
};
pub(super) use acp_contracts::{MessageRole, PermissionDecision, StreamEvent, StreamEventPayload};
use acp_mock::{MockConfig, spawn_with_shutdown_task};
pub(super) use acp_web_backend::ServerConfig;
use acp_web_backend::{AppState, serve_with_shutdown as serve_backend_with_shutdown};
pub(super) use anyhow::{Context, Result};
use eventsource_stream::Eventsource;
use futures_util::{Stream, StreamExt};
pub(super) use reqwest::{Client, StatusCode};
pub(super) use tokio::time::sleep;
use tokio::{net::TcpListener, sync::oneshot, task::JoinHandle};

pub(super) type SseStream = Pin<Box<dyn Stream<Item = Result<StreamEvent>> + Send>>;

pub(super) async fn expect_next_event(stream: &mut SseStream) -> Result<StreamEvent> {
    let next = tokio::time::timeout(Duration::from_secs(2), stream.next())
        .await
        .context("timed out waiting for SSE event")?;
    next.context("SSE stream ended unexpectedly")?
}

pub(super) struct TestStack {
    pub(super) backend_url: String,
    pub(super) client: Client,
    backend_shutdown: Option<oneshot::Sender<()>>,
    mock_shutdown: Option<oneshot::Sender<()>>,
}

impl TestStack {
    pub(super) async fn spawn(mut backend_config: ServerConfig) -> Result<Self> {
        let client = Client::builder().build().context("building test client")?;
        let mock_shutdown = maybe_spawn_mock_server(&mut backend_config).await?;
        let (backend_url, backend_shutdown_tx) = spawn_backend_server(backend_config).await?;
        wait_for_stack_health(&client, &backend_url).await?;

        Ok(Self {
            backend_url,
            client,
            backend_shutdown: Some(backend_shutdown_tx),
            mock_shutdown,
        })
    }

    pub(super) async fn create_session(&self, token: &str) -> Result<CreateSessionResponse> {
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

    pub(super) async fn submit_prompt(
        &self,
        token: &str,
        session_id: &str,
        prompt: &str,
    ) -> Result<()> {
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

    pub(super) async fn close_session(&self, token: &str, session_id: &str) -> Result<()> {
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

    pub(super) async fn cancel_turn(
        &self,
        token: &str,
        session_id: &str,
    ) -> Result<CancelTurnResponse> {
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

    pub(super) async fn resolve_permission(
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

    pub(super) async fn session_history(
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

    pub(super) async fn open_events(&self, token: &str, session_id: &str) -> Result<SseStream> {
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

async fn maybe_spawn_mock_server(
    backend_config: &mut ServerConfig,
) -> Result<Option<oneshot::Sender<()>>> {
    if !backend_config.acp_server.is_empty() {
        return Ok(None);
    }

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
    Ok(Some(mock_shutdown_tx))
}

async fn spawn_backend_server(
    backend_config: ServerConfig,
) -> Result<(String, oneshot::Sender<()>)> {
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
        if let Err(error) = serve_backend_with_shutdown(backend_listener, state, shutdown).await {
            eprintln!("test backend stopped: {error}");
        }
    });

    Ok((format!("http://{backend_address}"), backend_shutdown_tx))
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

pub(super) async fn spawn_direct_mock_server()
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

pub(super) async fn spawn_direct_backend_server(
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

pub(super) async fn spawn_graceful_mock_server()
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

pub(super) async fn spawn_graceful_backend_server(
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
