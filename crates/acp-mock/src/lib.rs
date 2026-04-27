mod prompt;
pub mod runtime;
pub mod support;

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
    prompt_requires_permission, prompt_should_fail, prompt_text, reply_for, response_delay_for,
    wait_for_cancel,
};
use tokio::{
    net::{
        TcpListener, TcpStream,
        tcp::{OwnedReadHalf, OwnedWriteHalf},
    },
    sync::watch,
};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use tracing::{error, info};

pub use prompt::{MANUAL_CANCEL_TRIGGER, MANUAL_FAILURE_TRIGGER, MANUAL_PERMISSION_TRIGGER};
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

        if prompt_should_fail(&prompt) {
            return Err(acp::Error::internal_error());
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
        _ => None,
    }
}

#[rustfmt::skip]
fn log_connection_result(result: Result<(), acp::Error>) { if let Err(error) = result { error!("mock ACP connection failed: {error}"); } }

#[rustfmt::skip]
fn spawn_connection_task(stream: TcpStream, state: Arc<MockServerState>) { tokio::spawn(async move { log_connection_result(handle_connection(stream, state).await); }); }

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
                spawn_connection_task(stream, state);
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

#[derive(Clone)]
struct MockDispatchHandler {
    agent: MockAgent,
}

impl MockDispatchHandler {
    fn new(agent: MockAgent) -> Self {
        Self { agent }
    }

    async fn handle_initialize_request(
        &self,
        dispatch: acp::Dispatch,
    ) -> Result<Option<acp::Dispatch>, acp::Error> {
        match dispatch.into_request::<schema::InitializeRequest>()? {
            Ok((args, responder)) => {
                respond_initialize_request(self.agent.clone(), args, responder).await?;
                Ok(None)
            }
            Err(dispatch) => Ok(Some(dispatch)),
        }
    }

    async fn handle_authenticate_request(
        &self,
        dispatch: acp::Dispatch,
    ) -> Result<Option<acp::Dispatch>, acp::Error> {
        match dispatch.into_request::<schema::AuthenticateRequest>()? {
            Ok((args, responder)) => {
                respond_authenticate_request(self.agent.clone(), args, responder).await?;
                Ok(None)
            }
            Err(dispatch) => Ok(Some(dispatch)),
        }
    }

    async fn handle_new_session_request(
        &self,
        dispatch: acp::Dispatch,
        connection: acp::ConnectionTo<acp::Client>,
    ) -> Result<Option<acp::Dispatch>, acp::Error> {
        match dispatch.into_request::<schema::NewSessionRequest>()? {
            Ok((args, responder)) => {
                respond_new_session_request(self.agent.clone(), args, responder, connection)
                    .await?;
                Ok(None)
            }
            Err(dispatch) => Ok(Some(dispatch)),
        }
    }

    async fn handle_load_session_request(
        &self,
        dispatch: acp::Dispatch,
    ) -> Result<Option<acp::Dispatch>, acp::Error> {
        match dispatch.into_request::<schema::LoadSessionRequest>()? {
            Ok((args, responder)) => {
                respond_load_session_request(self.agent.clone(), args, responder).await?;
                Ok(None)
            }
            Err(dispatch) => Ok(Some(dispatch)),
        }
    }

    async fn handle_prompt_request(
        &self,
        dispatch: acp::Dispatch,
        connection: acp::ConnectionTo<acp::Client>,
    ) -> Result<Option<acp::Dispatch>, acp::Error> {
        match dispatch.into_request::<schema::PromptRequest>()? {
            Ok((args, responder)) => {
                respond_prompt_request(self.agent.clone(), args, responder, connection).await?;
                Ok(None)
            }
            Err(dispatch) => Ok(Some(dispatch)),
        }
    }

    async fn handle_set_session_mode_request(
        &self,
        dispatch: acp::Dispatch,
    ) -> Result<Option<acp::Dispatch>, acp::Error> {
        match dispatch.into_request::<schema::SetSessionModeRequest>()? {
            Ok((args, responder)) => {
                respond_set_session_mode_request(self.agent.clone(), args, responder).await?;
                Ok(None)
            }
            Err(dispatch) => Ok(Some(dispatch)),
        }
    }

    async fn handle_cancel_notification(
        &self,
        dispatch: acp::Dispatch,
    ) -> Result<Option<acp::Dispatch>, acp::Error> {
        match dispatch.into_notification::<schema::CancelNotification>()? {
            Ok(args) => {
                handle_cancel_notification(self.agent.clone(), args).await?;
                Ok(None)
            }
            Err(dispatch) => Ok(Some(dispatch)),
        }
    }
}

impl acp::HandleDispatchFrom<acp::Client> for MockDispatchHandler {
    fn describe_chain(&self) -> impl std::fmt::Debug {
        "MockDispatchHandler"
    }

    async fn handle_dispatch_from(
        &mut self,
        dispatch: acp::Dispatch,
        connection: acp::ConnectionTo<acp::Client>,
    ) -> Result<acp::Handled<acp::Dispatch>, acp::Error> {
        // Keep the handler monomorphic so the launcher binary does not pull in a deeply nested
        // Builder<..., ChainedHandler<...>> type for every registered ACP mock callback.
        let Some(dispatch) = self.handle_initialize_request(dispatch).await? else {
            return Ok(acp::Handled::Yes);
        };
        let Some(dispatch) = self.handle_authenticate_request(dispatch).await? else {
            return Ok(acp::Handled::Yes);
        };
        let Some(dispatch) = self
            .handle_new_session_request(dispatch, connection.clone())
            .await?
        else {
            return Ok(acp::Handled::Yes);
        };
        let Some(dispatch) = self.handle_load_session_request(dispatch).await? else {
            return Ok(acp::Handled::Yes);
        };
        let Some(dispatch) = self
            .handle_prompt_request(dispatch, connection.clone())
            .await?
        else {
            return Ok(acp::Handled::Yes);
        };
        let Some(dispatch) = self.handle_set_session_mode_request(dispatch).await? else {
            return Ok(acp::Handled::Yes);
        };
        let Some(dispatch) = self.handle_cancel_notification(dispatch).await? else {
            return Ok(acp::Handled::Yes);
        };
        Ok(acp::Handled::No {
            message: dispatch,
            retry: false,
        })
    }
}

fn build_mock_agent_builder(
    agent: MockAgent,
) -> acp::Builder<acp::Agent, MockDispatchHandler, acp::NullRun> {
    acp::Builder::new_with(acp::Agent, MockDispatchHandler::new(agent)).name("acp-mock")
}

#[rustfmt::skip]
async fn connect_mock_agent(reader: OwnedReadHalf, writer: OwnedWriteHalf, agent: MockAgent) -> Result<(), acp::Error> { build_mock_agent_builder(agent).connect_to(acp::ByteStreams::new(writer.compat_write(), reader.compat())).await }

#[rustfmt::skip]
async fn handle_connection(stream: TcpStream, state: Arc<MockServerState>) -> Result<(), acp::Error> { let (reader, writer) = stream.into_split(); connect_mock_agent(reader, writer, MockAgent::new(state)).await }

#[cfg(test)]
mod coverage_tests {
    use std::{net::SocketAddr, sync::Arc, time::Duration};

    use agent_client_protocol::{HandleDispatchFrom, JsonRpcMessage, schema};
    use tokio::{
        io::{duplex, split},
        net::{TcpListener, TcpStream},
        task::JoinHandle,
    };
    use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

    use super::{
        MANUAL_PERMISSION_TRIGGER, MockAgent, MockConfig, MockDispatchHandler, MockServerState,
        prompt_response_for_permission_outcome, spawn_with_shutdown_task,
    };
    use agent_client_protocol as acp;

    #[derive(Debug, Clone)]
    struct RoundtripClient {
        reply: Arc<tokio::sync::Mutex<String>>,
        permission_response: schema::RequestPermissionResponse,
    }

    impl RoundtripClient {
        fn new() -> Self {
            Self::with_permission_response(schema::RequestPermissionResponse::new(
                schema::RequestPermissionOutcome::Selected(schema::SelectedPermissionOutcome::new(
                    "allow_once",
                )),
            ))
        }

        fn with_permission_response(
            permission_response: schema::RequestPermissionResponse,
        ) -> Self {
            Self {
                reply: Arc::new(tokio::sync::Mutex::new(String::new())),
                permission_response,
            }
        }

        async fn reply_text(&self) -> String {
            self.reply.lock().await.clone()
        }

        async fn request_permission(
            &self,
            _args: schema::RequestPermissionRequest,
        ) -> acp::Result<schema::RequestPermissionResponse> {
            Ok(self.permission_response.clone())
        }

        #[rustfmt::skip]
        async fn session_notification(&self, args: schema::SessionNotification) -> acp::Result<()> { if let schema::SessionUpdate::AgentMessageChunk(chunk) = args.update { self.reply.lock().await.push_str(&content_text(chunk.content)); } Ok(()) }
    }

    async fn spawn_roundtrip_server() -> (
        SocketAddr,
        tokio::sync::oneshot::Sender<()>,
        JoinHandle<std::io::Result<()>>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test listener should bind");
        let address = listener
            .local_addr()
            .expect("test listener should expose a local address");
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        let server = spawn_with_shutdown_task(
            listener,
            MockConfig {
                response_delay: Duration::from_millis(1),
                startup_hints: false,
            },
            async move {
                let _ = shutdown_rx.await;
            },
        );

        (address, shutdown_tx, server)
    }

    async fn connect_roundtrip_stream(address: SocketAddr) -> TcpStream {
        TcpStream::connect(address)
            .await
            .expect("test client should connect")
    }

    fn roundtrip_initialize_request() -> schema::InitializeRequest {
        schema::InitializeRequest::new(schema::ProtocolVersion::V1).client_info(
            schema::Implementation::new("acp-mock-unit-test", env!("CARGO_PKG_VERSION"))
                .title("ACP Mock Unit Test"),
        )
    }

    async fn initialize_roundtrip_connection(
        connection: &acp::ConnectionTo<acp::Agent>,
    ) -> acp::Result<()> {
        connection
            .send_request(roundtrip_initialize_request())
            .block_task()
            .await?;
        Ok(())
    }

    async fn finish_roundtrip(
        shutdown_tx: tokio::sync::oneshot::Sender<()>,
        server: JoinHandle<std::io::Result<()>>,
    ) {
        shutdown_tx
            .send(())
            .expect("test shutdown signals should send");
        tokio::time::timeout(Duration::from_secs(1), server)
            .await
            .expect("test server roundtrips should shut down promptly")
            .expect("test server tasks should join")
            .expect("test server roundtrips should succeed");
    }

    async fn execute_load_session_roundtrip(
        connection: acp::ConnectionTo<acp::Agent>,
        working_dir: std::path::PathBuf,
        prompt: String,
        reply_client: RoundtripClient,
    ) -> acp::Result<(schema::LoadSessionResponse, String)> {
        initialize_roundtrip_connection(&connection).await?;
        let loaded = connection
            .send_request(schema::LoadSessionRequest::new("mock_0", working_dir))
            .block_task()
            .await?;
        connection
            .send_request(schema::PromptRequest::new("mock_0", vec![prompt.into()]))
            .block_task()
            .await?;
        Ok((loaded, reply_client.reply_text().await))
    }

    async fn execute_new_session_roundtrip(
        connection: acp::ConnectionTo<acp::Agent>,
        working_dir: std::path::PathBuf,
        prompt: String,
        reply_client: RoundtripClient,
    ) -> acp::Result<(schema::NewSessionResponse, String)> {
        initialize_roundtrip_connection(&connection).await?;
        connection
            .send_request(schema::AuthenticateRequest::new("local"))
            .block_task()
            .await?;
        let created = connection
            .send_request(schema::NewSessionRequest::new(working_dir))
            .block_task()
            .await?;
        connection
            .send_request(schema::PromptRequest::new(
                created.session_id.clone(),
                vec![prompt.into()],
            ))
            .block_task()
            .await?;
        connection
            .send_request(schema::SetSessionModeRequest::new(
                created.session_id.clone(),
                "default",
            ))
            .block_task()
            .await?;
        #[rustfmt::skip]
        connection.send_notification(schema::CancelNotification::new(created.session_id.clone()))?;
        Ok((created, reply_client.reply_text().await))
    }

    async fn run_mock_client_roundtrip(
        prompt: &str,
    ) -> acp::Result<(schema::LoadSessionResponse, String)> {
        let (address, shutdown_tx, server) = spawn_roundtrip_server().await;
        let stream = connect_roundtrip_stream(address).await;
        let (reader, writer) = stream.into_split();
        let client = RoundtripClient::new();
        let notification_client = client.clone();
        let reply_client = client.clone();
        let prompt = prompt.to_string();
        let working_dir = std::env::current_dir().expect("current directory should be available");

        let result = acp::Client
            .builder()
            .name("acp-mock-unit-test")
            .on_receive_notification(
                async move |args: schema::SessionNotification, _cx| {
                    notification_client.session_notification(args).await
                },
                acp::on_receive_notification!(),
            )
            .connect_with(
                acp::ByteStreams::new(writer.compat_write(), reader.compat()),
                move |connection: acp::ConnectionTo<acp::Agent>| {
                    execute_load_session_roundtrip(connection, working_dir, prompt, reply_client)
                },
            )
            .await?;

        finish_roundtrip(shutdown_tx, server).await;
        Ok(result)
    }

    async fn run_mock_new_session_roundtrip(
        prompt: &str,
    ) -> acp::Result<(schema::NewSessionResponse, String)> {
        let (address, shutdown_tx, server) = spawn_roundtrip_server().await;
        let stream = connect_roundtrip_stream(address).await;
        let (reader, writer) = stream.into_split();
        let client = RoundtripClient::new();
        let request_client = client.clone();
        let notification_client = client.clone();
        let reply_client = client.clone();
        let prompt = prompt.to_string();
        let working_dir = std::env::current_dir().expect("current directory should be available");

        let result = acp::Client
            .builder()
            .name("acp-mock-unit-test")
            .on_receive_request(
                async move |args: schema::RequestPermissionRequest, responder, _cx| {
                    responder.respond_with_result(request_client.request_permission(args).await)
                },
                acp::on_receive_request!(),
            )
            .on_receive_notification(
                async move |args: schema::SessionNotification, _cx| {
                    notification_client.session_notification(args).await
                },
                acp::on_receive_notification!(),
            )
            .connect_with(
                acp::ByteStreams::new(writer.compat_write(), reader.compat()),
                move |connection: acp::ConnectionTo<acp::Agent>| {
                    execute_new_session_roundtrip(connection, working_dir, prompt, reply_client)
                },
            )
            .await?;

        finish_roundtrip(shutdown_tx, server).await;
        Ok(result)
    }

    #[rustfmt::skip]
    fn content_text(content: schema::ContentBlock) -> String { match content { schema::ContentBlock::Text(text) => text.text, schema::ContentBlock::Image(_) => "<image>".to_string(), schema::ContentBlock::Audio(_) => "<audio>".to_string(), schema::ContentBlock::ResourceLink(link) => link.uri, schema::ContentBlock::Resource(_) => "<resource>".to_string(), _ => "<unsupported>".to_string() } }

    #[test]
    fn allow_once_permissions_continue_the_prompt_flow() {
        assert_eq!(
            prompt_response_for_permission_outcome(schema::RequestPermissionOutcome::Selected(
                schema::SelectedPermissionOutcome::new("allow_once"),
            )),
            None
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn mock_agent_handles_load_session_and_prompt_requests_over_acp() {
        let (loaded, reply) = run_mock_client_roundtrip("hello")
            .await
            .expect("mock ACP roundtrip should succeed");

        assert_eq!(loaded, schema::LoadSessionResponse::new());
        assert!(reply.starts_with("mock assistant: I received `hello`"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn mock_agent_handles_authenticate_new_session_and_permission_requests_over_acp() {
        let (created, reply) = run_mock_new_session_roundtrip(MANUAL_PERMISSION_TRIGGER)
            .await
            .expect("mock ACP roundtrip should succeed");

        assert_eq!(created.session_id.to_string(), "mock_0");
        assert!(reply.starts_with("mock assistant: I received `"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn mock_dispatch_handler_handles_cancel_notifications_directly() {
        let state = Arc::new(MockServerState::new(MockConfig::default()));
        let session_id = state.next_session_id();
        let session = state.session_state(&session_id);
        let (cancel_rx, generation) = session.subscribe_cancel();
        let mut handler = MockDispatchHandler::new(MockAgent::new(state));
        let dispatch = acp::Dispatch::Notification(
            schema::CancelNotification::new(session_id)
                .to_untyped_message()
                .expect("cancel notification should serialize"),
        );
        let (_client, server) = duplex(64);
        let (reader, writer) = split(server);
        let description = format!("{:?}", handler.describe_chain());
        let result = acp::Agent
            .builder()
            .connect_with(
                acp::ByteStreams::new(writer.compat_write(), reader.compat()),
                async move |connection: acp::ConnectionTo<acp::Client>| {
                    handler.handle_dispatch_from(dispatch, connection).await
                },
            )
            .await
            .expect("handler should run over a direct test connection");

        assert_eq!(description, "\"MockDispatchHandler\"");
        assert!(matches!(result, acp::Handled::Yes));
        assert_eq!(*cancel_rx.borrow(), generation + 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn spawn_with_shutdown_task_accepts_connections_before_shutdown() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test listener should bind");
        let address = listener
            .local_addr()
            .expect("test listener should expose a local address");
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        let server = spawn_with_shutdown_task(
            listener,
            MockConfig {
                response_delay: Duration::from_millis(1),
                startup_hints: false,
            },
            async move {
                let _ = shutdown_rx.await;
            },
        );

        let stream = TcpStream::connect(address)
            .await
            .expect("test clients should connect");
        drop(stream);
        shutdown_tx
            .send(())
            .expect("test shutdown signals should send");

        tokio::time::timeout(Duration::from_secs(1), server)
            .await
            .expect("the server should stop promptly")
            .expect("the background task should join")
            .expect("the server should shut down cleanly");
    }
}
