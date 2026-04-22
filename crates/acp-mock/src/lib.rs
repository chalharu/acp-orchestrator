mod prompt;
pub mod runtime;

#[cfg(test)]
mod tests;

use std::{
    collections::HashMap,
    future::Future,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use agent_client_protocol::{self as acp, schema};
use prompt::{
    prompt_requires_permission, prompt_text, reply_for, response_delay_for, wait_for_cancel,
};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::watch,
};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use tracing::{error, info};

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
    next_session_id: AtomicU64,
    next_tool_call_id: AtomicU64,
    sessions: Mutex<HashMap<String, Arc<MockSessionState>>>,
}

impl MockServerState {
    fn new(config: MockConfig) -> Self {
        Self {
            config,
            next_session_id: AtomicU64::new(0),
            next_tool_call_id: AtomicU64::new(0),
            sessions: Mutex::new(HashMap::new()),
        }
    }

    fn next_session_id(&self) -> String {
        let next = self.next_session_id.fetch_add(1, Ordering::Relaxed);
        let session_id = format!("mock_{next}");
        self.sessions
            .lock()
            .expect("mock sessions mutex should not be poisoned")
            .entry(session_id.clone())
            .or_insert_with(|| Arc::new(MockSessionState::new()));
        session_id
    }

    fn next_tool_call_id(&self) -> String {
        let next = self.next_tool_call_id.fetch_add(1, Ordering::Relaxed);
        format!("tool_{next}")
    }

    fn session_state(&self, session_id: &str) -> Arc<MockSessionState> {
        self.sessions
            .lock()
            .expect("mock sessions mutex should not be poisoned")
            .entry(session_id.to_string())
            .or_insert_with(|| Arc::new(MockSessionState::new()))
            .clone()
    }
}

#[derive(Debug)]
struct MockSessionState {
    cancel_generation: AtomicU64,
    cancel_tx: watch::Sender<u64>,
}

impl MockSessionState {
    fn new() -> Self {
        let (cancel_tx, _) = watch::channel(0);
        Self {
            cancel_generation: AtomicU64::new(0),
            cancel_tx,
        }
    }

    fn subscribe_cancel(&self) -> (watch::Receiver<u64>, u64) {
        let cancel_rx = self.cancel_tx.subscribe();
        let generation = *cancel_rx.borrow();
        (cancel_rx, generation)
    }

    fn cancel(&self) {
        let next_generation = self.cancel_generation.fetch_add(1, Ordering::Relaxed) + 1;
        let _ = self.cancel_tx.send(next_generation);
    }
}

#[async_trait::async_trait]
trait SessionUpdateNotifier {
    async fn send_session_update(
        &self,
        notification: schema::SessionNotification,
    ) -> Result<(), acp::Error>;
}

#[async_trait::async_trait]
trait PermissionRequester {
    async fn request_permission(
        &self,
        request: schema::RequestPermissionRequest,
    ) -> Result<schema::RequestPermissionResponse, acp::Error>;
}

#[derive(Clone)]
struct ConnectionClientAdapter {
    connection: acp::ConnectionTo<acp::Client>,
}

impl ConnectionClientAdapter {
    fn new(connection: acp::ConnectionTo<acp::Client>) -> Self {
        Self { connection }
    }
}

#[async_trait::async_trait]
impl SessionUpdateNotifier for ConnectionClientAdapter {
    async fn send_session_update(
        &self,
        notification: schema::SessionNotification,
    ) -> Result<(), acp::Error> {
        self.connection.send_notification(notification)
    }
}

#[async_trait::async_trait]
impl PermissionRequester for ConnectionClientAdapter {
    async fn request_permission(
        &self,
        request: schema::RequestPermissionRequest,
    ) -> Result<schema::RequestPermissionResponse, acp::Error> {
        self.connection.send_request(request).block_task().await
    }
}

#[derive(Clone)]
struct MockAgent {
    state: Arc<MockServerState>,
}

impl MockAgent {
    fn new(state: Arc<MockServerState>) -> Self {
        Self { state }
    }

    async fn initialize(
        &self,
        _arguments: schema::InitializeRequest,
    ) -> Result<schema::InitializeResponse, acp::Error> {
        Ok(
            schema::InitializeResponse::new(schema::ProtocolVersion::V1).agent_info(
                schema::Implementation::new("acp-mock", env!("CARGO_PKG_VERSION"))
                    .title("ACP Mock"),
            ),
        )
    }

    async fn authenticate(
        &self,
        _arguments: schema::AuthenticateRequest,
    ) -> Result<schema::AuthenticateResponse, acp::Error> {
        Ok(schema::AuthenticateResponse::default())
    }

    async fn new_session<N: SessionUpdateNotifier + Sync>(
        &self,
        _arguments: schema::NewSessionRequest,
        notifier: &N,
    ) -> Result<schema::NewSessionResponse, acp::Error> {
        let session_id = self.state.next_session_id();
        if self.state.config.startup_hints {
            notifier
                .send_session_update(schema::SessionNotification::new(
                    session_id.clone(),
                    schema::SessionUpdate::AgentMessageChunk(schema::ContentChunk::new(
                        startup_hint_message().into(),
                    )),
                ))
                .await?;
        }
        Ok(schema::NewSessionResponse::new(session_id))
    }

    async fn load_session(
        &self,
        arguments: schema::LoadSessionRequest,
    ) -> Result<schema::LoadSessionResponse, acp::Error> {
        let _ = self.state.session_state(&arguments.session_id.to_string());
        Ok(schema::LoadSessionResponse::new())
    }

    async fn prompt_permission_response<P: PermissionRequester + Sync>(
        &self,
        session_id: &str,
        prompt: &str,
        requester: &P,
    ) -> Result<Option<schema::PromptResponse>, acp::Error> {
        if !prompt_requires_permission(prompt) {
            return Ok(None);
        }

        let outcome = requester
            .request_permission(permission_request(
                session_id.to_string(),
                self.state.next_tool_call_id(),
            ))
            .await?
            .outcome;
        Ok(prompt_response_for_permission_outcome(outcome))
    }

    async fn send_prompt_reply<N: SessionUpdateNotifier + Sync>(
        &self,
        notifier: &N,
        session_id: String,
        prompt: &str,
    ) -> Result<schema::PromptResponse, acp::Error> {
        notifier
            .send_session_update(schema::SessionNotification::new(
                session_id,
                schema::SessionUpdate::AgentMessageChunk(schema::ContentChunk::new(
                    reply_for(prompt).into(),
                )),
            ))
            .await?;
        Ok(end_turn_prompt_response())
    }

    async fn prompt<N, P>(
        &self,
        arguments: schema::PromptRequest,
        notifier: &N,
        requester: &P,
    ) -> Result<schema::PromptResponse, acp::Error>
    where
        N: SessionUpdateNotifier + Sync,
        P: PermissionRequester + Sync,
    {
        let prompt = prompt_text(&arguments.prompt);
        let session_id = arguments.session_id.to_string();
        let session_state = self.state.session_state(&session_id);
        let (mut cancel_rx, cancel_generation) = session_state.subscribe_cancel();

        if let Some(response) = self
            .prompt_permission_response(&session_id, &prompt, requester)
            .await?
        {
            return Ok(response);
        }

        if wait_for_cancel(
            &mut cancel_rx,
            cancel_generation,
            response_delay_for(&prompt, self.state.config.response_delay),
        )
        .await
        {
            return Ok(cancelled_prompt_response());
        }

        self.send_prompt_reply(notifier, session_id, &prompt).await
    }

    async fn cancel(&self, args: schema::CancelNotification) -> Result<(), acp::Error> {
        self.state
            .session_state(&args.session_id.to_string())
            .cancel();
        Ok(())
    }

    async fn set_session_mode(
        &self,
        _args: schema::SetSessionModeRequest,
    ) -> Result<schema::SetSessionModeResponse, acp::Error> {
        Ok(schema::SetSessionModeResponse::default())
    }
}

fn permission_request(
    session_id: String,
    tool_call_id: String,
) -> schema::RequestPermissionRequest {
    schema::RequestPermissionRequest::new(
        session_id,
        schema::ToolCallUpdate::new(
            tool_call_id,
            schema::ToolCallUpdateFields::new().title("read_text_file README.md"),
        ),
        vec![
            schema::PermissionOption::new(
                "allow_once",
                "Allow once",
                schema::PermissionOptionKind::AllowOnce,
            ),
            schema::PermissionOption::new(
                "reject_once",
                "Reject once",
                schema::PermissionOptionKind::RejectOnce,
            ),
        ],
    )
}

fn startup_hint_message() -> String {
    format!(
        "Bundled mock ready.\nTry `{MANUAL_PERMISSION_TRIGGER}` for a permission request or `{MANUAL_CANCEL_TRIGGER}` to test cancellation."
    )
}

fn cancelled_prompt_response() -> schema::PromptResponse {
    schema::PromptResponse::new(schema::StopReason::Cancelled)
}

fn end_turn_prompt_response() -> schema::PromptResponse {
    schema::PromptResponse::new(schema::StopReason::EndTurn)
}

fn prompt_response_for_permission_outcome(
    outcome: schema::RequestPermissionOutcome,
) -> Option<schema::PromptResponse> {
    match outcome {
        schema::RequestPermissionOutcome::Cancelled => Some(cancelled_prompt_response()),
        schema::RequestPermissionOutcome::Selected(selected)
            if selected.option_id.to_string() == "reject_once" =>
        {
            Some(end_turn_prompt_response())
        }
        schema::RequestPermissionOutcome::Selected(_) => None,
        _ => Some(cancelled_prompt_response()),
    }
}

fn log_connection_result(result: Result<(), acp::Error>) {
    if let Err(error) = result {
        error!("mock ACP connection failed: {error}");
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

    let state = Arc::new(MockServerState::new(config));
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            _ = &mut shutdown => return Ok(()),
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                let state = state.clone();
                tokio::spawn(async move {
                    log_connection_result(handle_connection(stream, state).await);
                });
            }
        }
    }
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

async fn respond_initialize_request(
    agent: MockAgent,
    args: schema::InitializeRequest,
    responder: acp::Responder<schema::InitializeResponse>,
) -> Result<(), acp::Error> {
    responder.respond_with_result(agent.initialize(args).await)
}

async fn respond_authenticate_request(
    agent: MockAgent,
    args: schema::AuthenticateRequest,
    responder: acp::Responder<schema::AuthenticateResponse>,
) -> Result<(), acp::Error> {
    responder.respond_with_result(agent.authenticate(args).await)
}

async fn respond_new_session_request(
    agent: MockAgent,
    args: schema::NewSessionRequest,
    responder: acp::Responder<schema::NewSessionResponse>,
    connection: acp::ConnectionTo<acp::Client>,
) -> Result<(), acp::Error> {
    let adapter = ConnectionClientAdapter::new(connection);
    responder.respond_with_result(agent.new_session(args, &adapter).await)
}

async fn respond_load_session_request(
    agent: MockAgent,
    args: schema::LoadSessionRequest,
    responder: acp::Responder<schema::LoadSessionResponse>,
) -> Result<(), acp::Error> {
    responder.respond_with_result(agent.load_session(args).await)
}

async fn respond_prompt_request(
    agent: MockAgent,
    args: schema::PromptRequest,
    responder: acp::Responder<schema::PromptResponse>,
    connection: acp::ConnectionTo<acp::Client>,
) -> Result<(), acp::Error> {
    let adapter = ConnectionClientAdapter::new(connection.clone());
    connection.spawn(async move {
        let result = agent.prompt(args, &adapter, &adapter).await;
        responder.respond_with_result(result)?;
        Ok(())
    })?;
    Ok(())
}

async fn respond_set_session_mode_request(
    agent: MockAgent,
    args: schema::SetSessionModeRequest,
    responder: acp::Responder<schema::SetSessionModeResponse>,
) -> Result<(), acp::Error> {
    responder.respond_with_result(agent.set_session_mode(args).await)
}

async fn handle_cancel_notification(
    agent: MockAgent,
    args: schema::CancelNotification,
) -> Result<(), acp::Error> {
    agent.cancel(args).await
}

macro_rules! register_request_handler {
    ($builder:expr, $agent:expr, $request:ty, $handler:ident) => {
        $builder.on_receive_request(
            {
                let agent = $agent.clone();
                async move |args: $request, responder, _cx| {
                    $handler(agent.clone(), args, responder).await
                }
            },
            acp::on_receive_request!(),
        )
    };
}

macro_rules! register_request_handler_with_connection {
    ($builder:expr, $agent:expr, $request:ty, $handler:ident) => {
        $builder.on_receive_request(
            {
                let agent = $agent.clone();
                async move |args: $request, responder, cx| {
                    $handler(agent.clone(), args, responder, cx).await
                }
            },
            acp::on_receive_request!(),
        )
    };
}

macro_rules! register_notification_handler {
    ($builder:expr, $agent:expr, $notification:ty, $handler:ident) => {
        $builder.on_receive_notification(
            {
                let agent = $agent.clone();
                async move |args: $notification, _cx| $handler(agent.clone(), args).await
            },
            acp::on_receive_notification!(),
        )
    };
}

macro_rules! build_mock_agent_handlers {
    ($builder:expr, $agent:expr) => {{
        let builder = register_request_handler!(
            $builder,
            $agent,
            schema::InitializeRequest,
            respond_initialize_request
        );
        let builder = register_request_handler!(
            builder,
            $agent,
            schema::AuthenticateRequest,
            respond_authenticate_request
        );
        let builder = register_request_handler_with_connection!(
            builder,
            $agent,
            schema::NewSessionRequest,
            respond_new_session_request
        );
        let builder = register_request_handler!(
            builder,
            $agent,
            schema::LoadSessionRequest,
            respond_load_session_request
        );
        let builder = register_request_handler_with_connection!(
            builder,
            $agent,
            schema::PromptRequest,
            respond_prompt_request
        );
        let builder = register_request_handler!(
            builder,
            $agent,
            schema::SetSessionModeRequest,
            respond_set_session_mode_request
        );
        register_notification_handler!(
            builder,
            $agent,
            schema::CancelNotification,
            handle_cancel_notification
        )
    }};
}

async fn handle_connection(
    stream: TcpStream,
    state: Arc<MockServerState>,
) -> Result<(), acp::Error> {
    let (reader, writer) = stream.into_split();
    let agent = MockAgent::new(state);
    let builder = build_mock_agent_handlers!(acp::Agent.builder().name("acp-mock"), agent);

    builder
        .connect_to(acp::ByteStreams::new(
            writer.compat_write(),
            reader.compat(),
        ))
        .await
}
