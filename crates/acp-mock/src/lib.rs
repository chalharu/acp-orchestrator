mod notifications;
mod prompt;
pub mod runtime;

#[cfg(test)]
mod tests;

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    future::Future,
    rc::Rc,
    time::Duration,
};

use agent_client_protocol::{self as acp, Client as _};
use notifications::{drain_permission_requests, drain_session_updates, log_connection_result};
use prompt::{
    prompt_requires_permission, prompt_text, reply_for, response_delay_for, wait_for_cancel,
};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::{mpsc, oneshot, watch},
};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use tracing::info;

pub use prompt::{MANUAL_CANCEL_TRIGGER, MANUAL_PERMISSION_TRIGGER};
pub use runtime::{MockAppError, run_with_args};

#[derive(Debug, Clone)]
pub struct MockConfig {
    pub response_delay: Duration,
    pub startup_hints: bool,
}

impl Default for MockConfig {
    fn default() -> Self {
        Self {
            response_delay: Duration::from_millis(120),
            startup_hints: false,
        }
    }
}

#[derive(Debug)]
struct MockServerState {
    config: MockConfig,
    next_session_id: Cell<u64>,
    next_tool_call_id: Cell<u64>,
    sessions: RefCell<HashMap<String, Rc<MockSessionState>>>,
}

impl MockServerState {
    fn new(config: MockConfig) -> Self {
        Self {
            config,
            next_session_id: Cell::new(0),
            next_tool_call_id: Cell::new(0),
            sessions: RefCell::new(HashMap::new()),
        }
    }

    fn next_session_id(&self) -> String {
        let next = self.next_session_id.get();
        self.next_session_id.set(next + 1);
        let session_id = format!("mock_{next}");
        self.sessions
            .borrow_mut()
            .entry(session_id.clone())
            .or_insert_with(|| Rc::new(MockSessionState::new()));
        session_id
    }

    fn next_tool_call_id(&self) -> String {
        let next = self.next_tool_call_id.get();
        self.next_tool_call_id.set(next + 1);
        format!("tool_{next}")
    }

    fn session_state(&self, session_id: &str) -> Rc<MockSessionState> {
        self.sessions
            .borrow_mut()
            .entry(session_id.to_string())
            .or_insert_with(|| Rc::new(MockSessionState::new()))
            .clone()
    }
}

type QueuedSessionNotification = (acp::SessionNotification, oneshot::Sender<()>);
type QueuedPermissionRequest = (
    acp::RequestPermissionRequest,
    oneshot::Sender<Result<acp::RequestPermissionResponse, acp::Error>>,
);

#[derive(Debug)]
struct MockSessionState {
    cancel_generation: Cell<u64>,
    cancel_tx: watch::Sender<u64>,
}

impl MockSessionState {
    fn new() -> Self {
        let (cancel_tx, _) = watch::channel(0);
        Self {
            cancel_generation: Cell::new(0),
            cancel_tx,
        }
    }

    fn subscribe_cancel(&self) -> (watch::Receiver<u64>, u64) {
        let cancel_rx = self.cancel_tx.subscribe();
        let generation = *cancel_rx.borrow();
        (cancel_rx, generation)
    }

    fn cancel(&self) {
        let next_generation = self.cancel_generation.get() + 1;
        self.cancel_generation.set(next_generation);
        let _ = self.cancel_tx.send(next_generation);
    }
}

#[async_trait::async_trait(?Send)]
trait SessionUpdateNotifier {
    async fn send_session_update(
        &self,
        notification: acp::SessionNotification,
    ) -> Result<(), acp::Error>;
}

#[async_trait::async_trait(?Send)]
trait PermissionRequester {
    async fn request_permission(
        &self,
        request: acp::RequestPermissionRequest,
    ) -> Result<acp::RequestPermissionResponse, acp::Error>;
}

struct MockAgent {
    state: Rc<MockServerState>,
    session_update_tx: mpsc::UnboundedSender<QueuedSessionNotification>,
    permission_request_tx: mpsc::UnboundedSender<QueuedPermissionRequest>,
}

impl MockAgent {
    fn new(
        state: Rc<MockServerState>,
        session_update_tx: mpsc::UnboundedSender<QueuedSessionNotification>,
        permission_request_tx: mpsc::UnboundedSender<QueuedPermissionRequest>,
    ) -> Self {
        Self {
            state,
            session_update_tx,
            permission_request_tx,
        }
    }

    async fn send_reply(&self, session_id: String, text: String) -> Result<(), acp::Error> {
        let (ack_tx, ack_rx) = oneshot::channel();
        self.session_update_tx
            .send((
                acp::SessionNotification::new(
                    session_id,
                    acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(text.into())),
                ),
                ack_tx,
            ))
            .map_err(|_| acp::Error::internal_error())?;
        ack_rx.await.map_err(|_| acp::Error::internal_error())
    }

    async fn request_permission(
        &self,
        session_id: String,
    ) -> Result<acp::RequestPermissionResponse, acp::Error> {
        let (ack_tx, ack_rx) = oneshot::channel();
        self.permission_request_tx
            .send((
                acp::RequestPermissionRequest::new(
                    session_id,
                    acp::ToolCallUpdate::new(
                        self.state.next_tool_call_id(),
                        acp::ToolCallUpdateFields::new().title("read_text_file README.md"),
                    ),
                    vec![
                        acp::PermissionOption::new(
                            "allow_once",
                            "Allow once",
                            acp::PermissionOptionKind::AllowOnce,
                        ),
                        acp::PermissionOption::new(
                            "reject_once",
                            "Reject once",
                            acp::PermissionOptionKind::RejectOnce,
                        ),
                    ],
                ),
                ack_tx,
            ))
            .map_err(|_| acp::Error::internal_error())?;
        ack_rx.await.map_err(|_| acp::Error::internal_error())?
    }
}

fn startup_hint_message() -> String {
    format!(
        "Bundled mock ready.\nTry `{MANUAL_PERMISSION_TRIGGER}` for a permission request or `{MANUAL_CANCEL_TRIGGER}` to test cancellation."
    )
}

#[async_trait::async_trait(?Send)]
impl acp::Agent for MockAgent {
    async fn initialize(
        &self,
        _arguments: acp::InitializeRequest,
    ) -> Result<acp::InitializeResponse, acp::Error> {
        Ok(
            acp::InitializeResponse::new(acp::ProtocolVersion::V1).agent_info(
                acp::Implementation::new("acp-mock", env!("CARGO_PKG_VERSION")).title("ACP Mock"),
            ),
        )
    }

    async fn authenticate(
        &self,
        _arguments: acp::AuthenticateRequest,
    ) -> Result<acp::AuthenticateResponse, acp::Error> {
        Ok(acp::AuthenticateResponse::default())
    }

    async fn new_session(
        &self,
        _arguments: acp::NewSessionRequest,
    ) -> Result<acp::NewSessionResponse, acp::Error> {
        let session_id = self.state.next_session_id();
        if self.state.config.startup_hints {
            self.send_reply(session_id.clone(), startup_hint_message())
                .await?;
        }
        Ok(acp::NewSessionResponse::new(session_id))
    }

    async fn load_session(
        &self,
        arguments: acp::LoadSessionRequest,
    ) -> Result<acp::LoadSessionResponse, acp::Error> {
        let _ = self.state.session_state(&arguments.session_id.to_string());
        Ok(acp::LoadSessionResponse::new())
    }

    async fn prompt(
        &self,
        arguments: acp::PromptRequest,
    ) -> Result<acp::PromptResponse, acp::Error> {
        let prompt = prompt_text(&arguments.prompt);
        let session_id = arguments.session_id.to_string();
        let session_state = self.state.session_state(&session_id);
        let (mut cancel_rx, cancel_generation) = session_state.subscribe_cancel();

        if prompt_requires_permission(&prompt) {
            match self.request_permission(session_id.clone()).await?.outcome {
                acp::RequestPermissionOutcome::Cancelled => {
                    return Ok(acp::PromptResponse::new(acp::StopReason::Cancelled));
                }
                acp::RequestPermissionOutcome::Selected(selected)
                    if selected.option_id.to_string() == "reject_once" =>
                {
                    return Ok(acp::PromptResponse::new(acp::StopReason::EndTurn));
                }
                acp::RequestPermissionOutcome::Selected(_) => {}
                _ => return Ok(acp::PromptResponse::new(acp::StopReason::Cancelled)),
            }
        }

        if wait_for_cancel(
            &mut cancel_rx,
            cancel_generation,
            response_delay_for(&prompt, self.state.config.response_delay),
        )
        .await
        {
            return Ok(acp::PromptResponse::new(acp::StopReason::Cancelled));
        }

        self.send_reply(session_id, reply_for(&prompt)).await?;
        Ok(acp::PromptResponse::new(acp::StopReason::EndTurn))
    }

    async fn cancel(&self, args: acp::CancelNotification) -> Result<(), acp::Error> {
        self.state
            .session_state(&args.session_id.to_string())
            .cancel();
        Ok(())
    }

    async fn set_session_mode(
        &self,
        _args: acp::SetSessionModeRequest,
    ) -> Result<acp::SetSessionModeResponse, acp::Error> {
        Ok(acp::SetSessionModeResponse::default())
    }
}

#[async_trait::async_trait(?Send)]
impl SessionUpdateNotifier for acp::AgentSideConnection {
    async fn send_session_update(
        &self,
        notification: acp::SessionNotification,
    ) -> Result<(), acp::Error> {
        self.session_notification(notification).await
    }
}

#[async_trait::async_trait(?Send)]
impl PermissionRequester for acp::AgentSideConnection {
    async fn request_permission(
        &self,
        request: acp::RequestPermissionRequest,
    ) -> Result<acp::RequestPermissionResponse, acp::Error> {
        acp::Client::request_permission(self, request).await
    }
}

pub async fn serve_with_shutdown<F>(
    listener: TcpListener,
    config: MockConfig,
    shutdown: F,
) -> std::io::Result<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    let address = listener.local_addr()?;
    info!("starting acp mock on {address}");

    let state = Rc::new(MockServerState::new(config));
    let local_set = tokio::task::LocalSet::new();
    local_set
        .run_until(async move {
            tokio::pin!(shutdown);

            loop {
                tokio::select! {
                    _ = &mut shutdown => return Ok(()),
                    accepted = listener.accept() => {
                        let (stream, _) = accepted?;
                        let state = state.clone();
                        tokio::task::spawn_local(async move {
                            log_connection_result(handle_connection(stream, state).await);
                        });
                    }
                }
            }
        })
        .await
}

pub fn spawn_with_shutdown_task<F>(
    listener: TcpListener,
    config: MockConfig,
    shutdown: F,
) -> tokio::task::JoinHandle<std::io::Result<()>>
where
    F: Future<Output = ()> + Send + 'static,
{
    tokio::task::spawn_blocking(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        runtime.block_on(async move { serve_with_shutdown(listener, config, shutdown).await })
    })
}

async fn handle_connection(
    stream: TcpStream,
    state: Rc<MockServerState>,
) -> Result<(), acp::Error> {
    let (reader, writer) = stream.into_split();
    let (session_update_tx, session_update_rx) = mpsc::unbounded_channel();
    let (permission_request_tx, permission_request_rx) = mpsc::unbounded_channel();
    let (conn, handle_io) = acp::AgentSideConnection::new(
        MockAgent::new(state, session_update_tx, permission_request_tx),
        writer.compat_write(),
        reader.compat(),
        |future| {
            tokio::task::spawn_local(future);
        },
    );
    let conn = Rc::new(conn);
    let session_updates_conn = conn.clone();
    let permission_requests_conn = conn.clone();

    tokio::task::spawn_local(async move {
        drain_session_updates(&*session_updates_conn, session_update_rx).await;
    });
    tokio::task::spawn_local(async move {
        drain_permission_requests(&*permission_requests_conn, permission_request_rx).await;
    });

    handle_io.await
}
