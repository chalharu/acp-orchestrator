use std::{future::pending, path::PathBuf, pin::Pin, sync::Arc, time::Duration};

use acp_app_support::{build_http_client_for_url, wait_for_health, wait_for_tcp_connect};
use acp_contracts::{
    BootstrapRegistrationRequest, BootstrapRegistrationResponse, CancelTurnResponse,
    CreateSessionResponse, PromptRequest, RenameSessionRequest, RenameSessionResponse,
    ResolvePermissionRequest, ResolvePermissionResponse, SessionListResponse,
};
pub(super) use acp_contracts::{MessageRole, PermissionDecision, StreamEvent, StreamEventPayload};
use acp_mock::{MockConfig, spawn_with_shutdown_task};
pub(super) use acp_web_backend::ServerConfig;
use acp_web_backend::{AppState, serve_with_shutdown as serve_backend_with_shutdown};
use acp_web_backend::{
    workspace_repository::WorkspaceRepository, workspace_store::SqliteWorkspaceRepository,
};
pub(super) use anyhow::{Context, Result};
use eventsource_stream::Eventsource;
use futures_util::{Stream, StreamExt};
pub(super) use reqwest::{Client, StatusCode};
pub(super) use tokio::time::sleep;
use tokio::{net::TcpListener, sync::oneshot, task::JoinHandle};

pub(super) type SseStream = Pin<Box<dyn Stream<Item = Result<StreamEvent>> + Send>>;

pub(super) fn test_state_dir() -> PathBuf {
    std::env::temp_dir().join(format!(
        "acp-session-api-roundtrip-{}",
        uuid::Uuid::new_v4().simple()
    ))
}

pub(super) async fn expect_next_event(stream: &mut SseStream) -> Result<StreamEvent> {
    let next = tokio::time::timeout(Duration::from_secs(2), stream.next())
        .await
        .context("timed out waiting for SSE event")?;
    next.context("SSE stream ended unexpectedly")?
}

pub(super) fn build_browser_client() -> Result<Client> {
    Client::builder()
        .cookie_store(true)
        .danger_accept_invalid_certs(true)
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .build()
        .context("building cookie-authenticated browser client")
}

pub(super) async fn load_browser_app_shell(client: &Client, backend_url: &str) -> Result<String> {
    client
        .get(format!("{backend_url}/app/"))
        .send()
        .await
        .context("loading the browser app shell")?
        .error_for_status()
        .context("browser app shell returned an error")?
        .text()
        .await
        .context("reading the browser app shell")
}

pub(super) async fn create_browser_session(
    client: &Client,
    backend_url: &str,
    csrf_token: &str,
) -> Result<CreateSessionResponse> {
    client
        .post(format!("{backend_url}/api/v1/sessions"))
        .header("x-csrf-token", csrf_token)
        .send()
        .await
        .context("creating a cookie-authenticated browser session")?
        .error_for_status()
        .context("cookie-authenticated browser session creation returned an error")?
        .json()
        .await
        .context("decoding the created browser session")
}

pub(super) async fn bootstrap_browser_account(
    client: &Client,
    backend_url: &str,
    csrf_token: &str,
    username: &str,
    password: &str,
) -> Result<BootstrapRegistrationResponse> {
    client
        .post(format!("{backend_url}/api/v1/bootstrap/register"))
        .header("x-csrf-token", csrf_token)
        .json(&BootstrapRegistrationRequest {
            username: username.to_string(),
            password: password.to_string(),
        })
        .send()
        .await
        .context("registering the bootstrap browser account")?
        .error_for_status()
        .context("bootstrap registration returned an error")?
        .json()
        .await
        .context("decoding the bootstrap registration response")
}

pub(super) async fn submit_browser_prompt(
    client: &Client,
    backend_url: &str,
    session_id: &str,
    csrf_token: &str,
    prompt: &str,
) -> Result<()> {
    client
        .post(format!(
            "{backend_url}/api/v1/sessions/{session_id}/messages"
        ))
        .header("x-csrf-token", csrf_token)
        .json(&PromptRequest {
            text: prompt.to_string(),
        })
        .send()
        .await
        .context("submitting a browser-authenticated prompt")?
        .error_for_status()
        .context("browser-authenticated prompt submission returned an error")?;
    Ok(())
}

pub(super) async fn resolve_browser_permission(
    client: &Client,
    backend_url: &str,
    session_id: &str,
    request_id: &str,
    csrf_token: &str,
    decision: PermissionDecision,
) -> Result<ResolvePermissionResponse> {
    client
        .post(format!(
            "{backend_url}/api/v1/sessions/{session_id}/permissions/{request_id}"
        ))
        .header("x-csrf-token", csrf_token)
        .json(&ResolvePermissionRequest { decision })
        .send()
        .await
        .context("resolving a browser-authenticated permission")?
        .error_for_status()
        .context("browser-authenticated permission resolution returned an error")?
        .json()
        .await
        .context("decoding the browser-authenticated permission resolution")
}

pub(super) async fn cancel_browser_turn(
    client: &Client,
    backend_url: &str,
    session_id: &str,
    csrf_token: &str,
) -> Result<CancelTurnResponse> {
    client
        .post(format!("{backend_url}/api/v1/sessions/{session_id}/cancel"))
        .header("x-csrf-token", csrf_token)
        .send()
        .await
        .context("cancelling a browser-authenticated turn")?
        .error_for_status()
        .context("browser-authenticated turn cancellation returned an error")?
        .json()
        .await
        .context("decoding the browser-authenticated cancel response")
}

pub(super) async fn open_cookie_events(
    client: &Client,
    backend_url: &str,
    session_id: &str,
) -> Result<SseStream> {
    let response = client
        .get(format!("{backend_url}/api/v1/sessions/{session_id}/events"))
        .send()
        .await
        .context("opening cookie-authenticated event stream")?
        .error_for_status()
        .context("cookie-authenticated event stream returned an error")?;

    let stream = response.bytes_stream().eventsource().map(|event| {
        let event = event.context("reading cookie-authenticated event")?;
        serde_json::from_str(&event.data).context("decoding cookie-authenticated event payload")
    });

    Ok(Box::pin(stream))
}

pub(super) fn extract_meta_content(document: &str, name: &str) -> Result<String> {
    let name_needle = format!(r#"name="{name}""#);
    let tag = document
        .lines()
        .find(|line| line.contains("<meta ") && line.contains(&name_needle))
        .context("meta tag was not present in the app shell")?
        .trim();
    let content_start = tag
        .find(r#"content=""#)
        .context("meta tag did not contain a content attribute")?
        + r#"content=""#.len();
    let content_end = tag[content_start..]
        .find('"')
        .context("meta tag content did not terminate")?
        + content_start;

    Ok(tag[content_start..content_end].to_string())
}

pub(super) struct TestStack {
    pub(super) backend_url: String,
    pub(super) client: Client,
    backend_shutdown: Option<oneshot::Sender<()>>,
    mock_shutdown: Option<oneshot::Sender<()>>,
}

impl TestStack {
    pub(super) async fn spawn(mut backend_config: ServerConfig) -> Result<Self> {
        let mock_shutdown = maybe_spawn_mock_server(&mut backend_config).await?;
        let (backend_url, backend_shutdown_tx) = spawn_backend_server(backend_config).await?;
        let client =
            build_http_client_for_url(&backend_url, None).context("building test client")?;
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

    pub(super) async fn rename_session(
        &self,
        token: &str,
        session_id: &str,
        title: &str,
    ) -> Result<RenameSessionResponse> {
        let response = self
            .client
            .patch(format!("{}/api/v1/sessions/{session_id}", self.backend_url))
            .bearer_auth(token)
            .json(&RenameSessionRequest {
                title: title.to_string(),
            })
            .send()
            .await
            .context("renaming test session")?
            .error_for_status()
            .context("rename session returned an error")?;
        response.json().await.context("decoding rename response")
    }

    pub(super) async fn delete_session(&self, token: &str, session_id: &str) -> Result<()> {
        self.client
            .delete(format!("{}/api/v1/sessions/{session_id}", self.backend_url))
            .bearer_auth(token)
            .send()
            .await
            .context("deleting test session")?
            .error_for_status()
            .context("delete session returned an error")?;
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

    pub(super) async fn session_snapshot(
        &self,
        token: &str,
        session_id: &str,
    ) -> Result<CreateSessionResponse> {
        let response = self
            .client
            .get(format!("{}/api/v1/sessions/{session_id}", self.backend_url))
            .bearer_auth(token)
            .send()
            .await
            .context("loading test session snapshot")?
            .error_for_status()
            .context("session snapshot request returned an error")?;
        response
            .json()
            .await
            .context("decoding session snapshot response")
    }

    pub(super) async fn list_sessions(&self, token: &str) -> Result<SessionListResponse> {
        let response = self
            .client
            .get(format!("{}/api/v1/sessions", self.backend_url))
            .bearer_auth(token)
            .send()
            .await
            .context("loading owned sessions")?
            .error_for_status()
            .context("owned session list request returned an error")?;
        response
            .json()
            .await
            .context("decoding owned session list response")
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
    let state = build_backend_state(backend_config)?;
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

    Ok((format!("https://{backend_address}"), backend_shutdown_tx))
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
    let state = build_backend_state(ServerConfig {
        session_cap: 8,
        acp_server: mock_address,
        startup_hints: false,
        state_dir: test_state_dir(),
        frontend_dist: None,
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

    Ok((format!("https://{address}"), handle))
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
    let state = build_backend_state(ServerConfig {
        session_cap: 8,
        acp_server: mock_address,
        startup_hints: false,
        state_dir: test_state_dir(),
        frontend_dist: None,
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

    Ok((format!("https://{address}"), shutdown_tx, handle))
}

fn build_backend_state(backend_config: ServerConfig) -> Result<AppState> {
    let workspace_repository: Arc<dyn WorkspaceRepository> = Arc::new(
        SqliteWorkspaceRepository::new(backend_config.state_dir.join("db.sqlite"))
            .context("building workspace repository")?,
    );
    AppState::new(backend_config, workspace_repository).context("building backend state")
}
