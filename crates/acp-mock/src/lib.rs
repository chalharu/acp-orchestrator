mod prompt;
pub mod runtime;
pub mod support;

#[cfg(test)]
mod tests;

use std::{
    collections::HashMap,
    future::Future,
    path::PathBuf,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use agent_client_protocol::{self as acp, ConnectTo, schema};
use futures_util::{
    Sink, Stream,
    future::{self, BoxFuture, Either},
    io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader},
    sink::unfold,
};
use prompt::{
    prompt_requires_permission, prompt_should_fail, prompt_text, prompt_uses_runtime_tools,
    reply_for, response_delay_for, wait_for_cancel,
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

pub use prompt::{
    MANUAL_CANCEL_TRIGGER, MANUAL_FAILURE_TRIGGER, MANUAL_PERMISSION_TRIGGER,
    MANUAL_RUNTIME_TOOLS_TRIGGER,
};
pub use runtime::{MockAppError, run_with_args};

const SESSION_UPDATE_FLUSH_DELAY: Duration = Duration::from_millis(10);

#[derive(Debug, Clone)]
pub struct MockConfig {
    pub response_delay: Duration,
    pub startup_hints: bool,
    pub auth_required: bool,
}

impl Default for MockConfig {
    fn default() -> Self {
        Self {
            response_delay: Duration::from_millis(120),
            startup_hints: false,
            auth_required: false,
        }
    }
}

const MOCK_AUTH_METHOD_ID: &str = "mock-agent-auth";

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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct ClientRuntimeCapabilities {
    read_text_file: bool,
    write_text_file: bool,
    terminal: bool,
}

impl ClientRuntimeCapabilities {
    fn from_client_capabilities(capabilities: &schema::ClientCapabilities) -> Self {
        Self {
            read_text_file: capabilities.fs.read_text_file,
            write_text_file: capabilities.fs.write_text_file,
            terminal: capabilities.terminal,
        }
    }

    fn supports_runtime_tools(self) -> bool {
        self.read_text_file && self.write_text_file && self.terminal
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

#[async_trait::async_trait]
trait RuntimeToolRequester {
    async fn read_text_file(
        &self,
        request: schema::ReadTextFileRequest,
    ) -> Result<schema::ReadTextFileResponse, acp::Error>;

    async fn write_text_file(
        &self,
        request: schema::WriteTextFileRequest,
    ) -> Result<schema::WriteTextFileResponse, acp::Error>;

    async fn create_terminal(
        &self,
        request: schema::CreateTerminalRequest,
    ) -> Result<schema::CreateTerminalResponse, acp::Error>;

    async fn terminal_output(
        &self,
        request: schema::TerminalOutputRequest,
    ) -> Result<schema::TerminalOutputResponse, acp::Error>;

    async fn wait_for_terminal_exit(
        &self,
        request: schema::WaitForTerminalExitRequest,
    ) -> Result<schema::WaitForTerminalExitResponse, acp::Error>;

    async fn kill_terminal(
        &self,
        request: schema::KillTerminalRequest,
    ) -> Result<schema::KillTerminalResponse, acp::Error>;

    async fn release_terminal(
        &self,
        request: schema::ReleaseTerminalRequest,
    ) -> Result<schema::ReleaseTerminalResponse, acp::Error>;
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

#[async_trait::async_trait]
impl RuntimeToolRequester for ConnectionClientAdapter {
    async fn read_text_file(
        &self,
        request: schema::ReadTextFileRequest,
    ) -> Result<schema::ReadTextFileResponse, acp::Error> {
        self.connection.send_request(request).block_task().await
    }

    async fn write_text_file(
        &self,
        request: schema::WriteTextFileRequest,
    ) -> Result<schema::WriteTextFileResponse, acp::Error> {
        self.connection.send_request(request).block_task().await
    }

    async fn create_terminal(
        &self,
        request: schema::CreateTerminalRequest,
    ) -> Result<schema::CreateTerminalResponse, acp::Error> {
        self.connection.send_request(request).block_task().await
    }

    async fn terminal_output(
        &self,
        request: schema::TerminalOutputRequest,
    ) -> Result<schema::TerminalOutputResponse, acp::Error> {
        self.connection.send_request(request).block_task().await
    }

    async fn wait_for_terminal_exit(
        &self,
        request: schema::WaitForTerminalExitRequest,
    ) -> Result<schema::WaitForTerminalExitResponse, acp::Error> {
        self.connection.send_request(request).block_task().await
    }

    async fn kill_terminal(
        &self,
        request: schema::KillTerminalRequest,
    ) -> Result<schema::KillTerminalResponse, acp::Error> {
        self.connection.send_request(request).block_task().await
    }

    async fn release_terminal(
        &self,
        request: schema::ReleaseTerminalRequest,
    ) -> Result<schema::ReleaseTerminalResponse, acp::Error> {
        self.connection.send_request(request).block_task().await
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RuntimeToolSummary {
    read_content: String,
    terminal_output: String,
    terminal_exit_code: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RuntimeTerminalSummary {
    output: String,
    exit_code: Option<u32>,
}

#[derive(Clone)]
struct MockAgent {
    state: Arc<MockServerState>,
    authenticated: Arc<AtomicBool>,
    runtime_capabilities: Arc<Mutex<ClientRuntimeCapabilities>>,
}

impl MockAgent {
    fn new(state: Arc<MockServerState>) -> Self {
        Self {
            state,
            authenticated: Arc::new(AtomicBool::new(false)),
            runtime_capabilities: Arc::new(Mutex::new(ClientRuntimeCapabilities::default())),
        }
    }

    fn is_authenticated(&self) -> bool {
        !self.state.config.auth_required || self.authenticated.load(Ordering::Relaxed)
    }

    fn mark_authenticated(&self) {
        self.authenticated.store(true, Ordering::Relaxed);
    }

    fn ensure_authenticated(&self) -> Result<(), acp::Error> {
        if self.is_authenticated() {
            Ok(())
        } else {
            Err(acp::Error::auth_required())
        }
    }

    fn set_runtime_capabilities(&self, capabilities: ClientRuntimeCapabilities) {
        *self
            .runtime_capabilities
            .lock()
            .expect("runtime capabilities mutex should not be poisoned") = capabilities;
    }

    fn supports_runtime_tools(&self) -> bool {
        self.runtime_capabilities
            .lock()
            .expect("runtime capabilities mutex should not be poisoned")
            .supports_runtime_tools()
    }

    async fn initialize(
        &self,
        arguments: schema::InitializeRequest,
    ) -> Result<schema::InitializeResponse, acp::Error> {
        self.set_runtime_capabilities(ClientRuntimeCapabilities::from_client_capabilities(
            &arguments.client_capabilities,
        ));
        let response = schema::InitializeResponse::new(schema::ProtocolVersion::V1)
            .agent_capabilities(schema::AgentCapabilities::new().load_session(true))
            .agent_info(
                schema::Implementation::new("acp-mock", env!("CARGO_PKG_VERSION"))
                    .title("ACP Mock"),
            );
        Ok(add_auth_methods(response, self.state.config.auth_required))
    }

    async fn authenticate(
        &self,
        arguments: schema::AuthenticateRequest,
    ) -> Result<schema::AuthenticateResponse, acp::Error> {
        if self.state.config.auth_required && arguments.method_id.to_string() != MOCK_AUTH_METHOD_ID
        {
            return Err(acp::Error::invalid_params());
        }
        self.mark_authenticated();
        Ok(schema::AuthenticateResponse::default())
    }

    async fn new_session<N: SessionUpdateNotifier + Sync>(
        &self,
        _arguments: schema::NewSessionRequest,
        notifier: &N,
    ) -> Result<schema::NewSessionResponse, acp::Error> {
        self.ensure_authenticated()?;
        let session_id = self.state.next_session_id();
        if self.state.config.startup_hints {
            let hint = startup_hint_message(self.supports_runtime_tools());
            notifier
                .send_session_update(schema::SessionNotification::new(
                    session_id.clone(),
                    schema::SessionUpdate::AgentMessageChunk(schema::ContentChunk::new(
                        hint.into(),
                    )),
                ))
                .await?;
            tokio::time::sleep(SESSION_UPDATE_FLUSH_DELAY).await;
        }
        Ok(schema::NewSessionResponse::new(session_id))
    }

    async fn load_session(
        &self,
        arguments: schema::LoadSessionRequest,
    ) -> Result<schema::LoadSessionResponse, acp::Error> {
        self.ensure_authenticated()?;
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
        tokio::time::sleep(SESSION_UPDATE_FLUSH_DELAY).await;
        Ok(end_turn_prompt_response())
    }

    async fn prompt_runtime_tools_response<C, N>(
        &self,
        session_id: &str,
        prompt: &str,
        notifier: &N,
        requester: &C,
    ) -> Result<Option<schema::PromptResponse>, acp::Error>
    where
        C: PermissionRequester + RuntimeToolRequester + Sync,
        N: SessionUpdateNotifier + Sync,
    {
        if !prompt_uses_runtime_tools(prompt) {
            return Ok(None);
        }
        if !self.supports_runtime_tools() {
            return self
                .send_runtime_tools_unavailable_reply(notifier, session_id.to_string())
                .await
                .map(Some);
        }
        self.execute_runtime_tools_prompt(session_id, notifier, requester)
            .await
            .map(Some)
    }

    async fn execute_runtime_tools_prompt<C, N>(
        &self,
        session_id: &str,
        notifier: &N,
        requester: &C,
    ) -> Result<schema::PromptResponse, acp::Error>
    where
        C: PermissionRequester + RuntimeToolRequester + Sync,
        N: SessionUpdateNotifier + Sync,
    {
        let tool_call_id = self.state.next_tool_call_id();
        self.send_runtime_tool_call(notifier, session_id, &tool_call_id)
            .await?;
        let permission_outcome =
            request_runtime_tools_permission(requester, session_id, &tool_call_id).await?;
        if let Some(response) = prompt_response_for_permission_outcome(permission_outcome) {
            self.send_runtime_tool_status(
                notifier,
                session_id,
                &tool_call_id,
                schema::ToolCallStatus::Failed,
            )
            .await?;
            return Ok(response);
        }

        let summary = self
            .run_runtime_tools_or_fail(session_id, notifier, requester, &tool_call_id)
            .await?;
        self.send_runtime_tool_status(
            notifier,
            session_id,
            &tool_call_id,
            schema::ToolCallStatus::Completed,
        )
        .await?;
        self.send_runtime_tool_reply(notifier, session_id.to_string(), summary)
            .await
    }

    async fn send_runtime_tool_call<N: SessionUpdateNotifier + Sync>(
        &self,
        notifier: &N,
        session_id: &str,
        tool_call_id: &str,
    ) -> Result<(), acp::Error> {
        notifier
            .send_session_update(schema::SessionNotification::new(
                session_id.to_string(),
                schema::SessionUpdate::ToolCall(
                    schema::ToolCall::new(tool_call_id.to_string(), "Verify ACP runtime tools")
                        .kind(schema::ToolKind::Execute)
                        .status(schema::ToolCallStatus::InProgress)
                        .raw_input(serde_json::json!({
                            "read": "/workspace/README.md",
                            "write": "/workspace/acp-mock-runtime-tools.txt",
                            "terminal": "/bin/printf",
                        })),
                ),
            ))
            .await
    }

    async fn send_runtime_tool_status<N: SessionUpdateNotifier + Sync>(
        &self,
        notifier: &N,
        session_id: &str,
        tool_call_id: &str,
        status: schema::ToolCallStatus,
    ) -> Result<(), acp::Error> {
        notifier
            .send_session_update(schema::SessionNotification::new(
                session_id.to_string(),
                schema::SessionUpdate::ToolCallUpdate(schema::ToolCallUpdate::new(
                    tool_call_id.to_string(),
                    schema::ToolCallUpdateFields::new().status(status),
                )),
            ))
            .await
    }

    async fn run_runtime_tools<C: RuntimeToolRequester + Sync>(
        &self,
        session_id: &str,
        requester: &C,
    ) -> Result<RuntimeToolSummary, acp::Error> {
        let read_content = read_runtime_fixture(session_id, requester).await?;
        write_runtime_fixture(session_id, requester).await?;
        let terminal = run_runtime_printf(session_id, requester).await?;
        kill_runtime_sleep(session_id, requester).await?;

        Ok(RuntimeToolSummary {
            read_content,
            terminal_output: terminal.output,
            terminal_exit_code: terminal.exit_code,
        })
    }

    async fn run_runtime_tools_or_fail<C, N>(
        &self,
        session_id: &str,
        notifier: &N,
        requester: &C,
        tool_call_id: &str,
    ) -> Result<RuntimeToolSummary, acp::Error>
    where
        C: RuntimeToolRequester + Sync,
        N: SessionUpdateNotifier + Sync,
    {
        match self.run_runtime_tools(session_id, requester).await {
            Ok(summary) => Ok(summary),
            Err(error) => {
                self.send_runtime_tool_status(
                    notifier,
                    session_id,
                    tool_call_id,
                    schema::ToolCallStatus::Failed,
                )
                .await?;
                Err(error)
            }
        }
    }

    async fn send_runtime_tool_reply<N: SessionUpdateNotifier + Sync>(
        &self,
        notifier: &N,
        session_id: String,
        summary: RuntimeToolSummary,
    ) -> Result<schema::PromptResponse, acp::Error> {
        notifier
            .send_session_update(schema::SessionNotification::new(
                session_id,
                schema::SessionUpdate::AgentMessageChunk(schema::ContentChunk::new(
                    runtime_tools_reply(summary).into(),
                )),
            ))
            .await?;
        tokio::time::sleep(SESSION_UPDATE_FLUSH_DELAY).await;
        Ok(end_turn_prompt_response())
    }

    async fn send_runtime_tools_unavailable_reply<N: SessionUpdateNotifier + Sync>(
        &self,
        notifier: &N,
        session_id: String,
    ) -> Result<schema::PromptResponse, acp::Error> {
        notifier
            .send_session_update(schema::SessionNotification::new(
                session_id,
                schema::SessionUpdate::AgentMessageChunk(schema::ContentChunk::new(
                    runtime_tools_unavailable_reply().into(),
                )),
            ))
            .await?;
        tokio::time::sleep(SESSION_UPDATE_FLUSH_DELAY).await;
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
        P: PermissionRequester + RuntimeToolRequester + Sync,
    {
        self.ensure_authenticated()?;
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

        if let Some(response) = self
            .prompt_runtime_tools_response(&session_id, &prompt, notifier, requester)
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
        self.ensure_authenticated()?;
        self.state
            .session_state(&args.session_id.to_string())
            .cancel();
        Ok(())
    }

    async fn set_session_mode(
        &self,
        _args: schema::SetSessionModeRequest,
    ) -> Result<schema::SetSessionModeResponse, acp::Error> {
        self.ensure_authenticated()?;
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
        permission_options(),
    )
}

fn runtime_tools_permission_request(
    session_id: String,
    tool_call_id: String,
) -> schema::RequestPermissionRequest {
    schema::RequestPermissionRequest::new(
        session_id,
        schema::ToolCallUpdate::new(
            tool_call_id,
            schema::ToolCallUpdateFields::new().title("verify runtime tools"),
        ),
        permission_options(),
    )
}

async fn request_runtime_tools_permission<C: PermissionRequester + Sync>(
    requester: &C,
    session_id: &str,
    tool_call_id: &str,
) -> Result<schema::RequestPermissionOutcome, acp::Error> {
    Ok(requester
        .request_permission(runtime_tools_permission_request(
            session_id.to_string(),
            tool_call_id.to_string(),
        ))
        .await?
        .outcome)
}

async fn read_runtime_fixture<C: RuntimeToolRequester + Sync>(
    session_id: &str,
    requester: &C,
) -> Result<String, acp::Error> {
    let response = requester
        .read_text_file(
            schema::ReadTextFileRequest::new(session_id.to_string(), "/workspace/README.md")
                .line(1)
                .limit(1),
        )
        .await?;
    Ok(response.content)
}

async fn write_runtime_fixture<C: RuntimeToolRequester + Sync>(
    session_id: &str,
    requester: &C,
) -> Result<(), acp::Error> {
    requester
        .write_text_file(schema::WriteTextFileRequest::new(
            session_id.to_string(),
            "/workspace/acp-mock-runtime-tools.txt",
            "created by acp-mock runtime tools\n",
        ))
        .await?;
    Ok(())
}

async fn run_runtime_printf<C: RuntimeToolRequester + Sync>(
    session_id: &str,
    requester: &C,
) -> Result<RuntimeTerminalSummary, acp::Error> {
    let terminal = requester
        .create_terminal(
            schema::CreateTerminalRequest::new(session_id.to_string(), "/bin/printf")
                .args(vec!["terminal-ok".to_string()])
                .cwd(PathBuf::from("/workspace"))
                .output_byte_limit(64),
        )
        .await?;
    let terminal_id = terminal.terminal_id.to_string();
    let exit = requester
        .wait_for_terminal_exit(schema::WaitForTerminalExitRequest::new(
            session_id.to_string(),
            terminal_id.clone(),
        ))
        .await?;
    let output = requester
        .terminal_output(schema::TerminalOutputRequest::new(
            session_id.to_string(),
            terminal_id.clone(),
        ))
        .await?;
    release_runtime_terminal(session_id, requester, terminal_id).await?;
    Ok(RuntimeTerminalSummary {
        output: output.output,
        exit_code: exit.exit_status.exit_code,
    })
}

async fn kill_runtime_sleep<C: RuntimeToolRequester + Sync>(
    session_id: &str,
    requester: &C,
) -> Result<(), acp::Error> {
    let sleep = requester
        .create_terminal(
            schema::CreateTerminalRequest::new(session_id.to_string(), "/bin/sleep")
                .args(vec!["5".to_string()])
                .cwd(PathBuf::from("/workspace")),
        )
        .await?;
    let sleep_id = sleep.terminal_id.to_string();
    requester
        .kill_terminal(schema::KillTerminalRequest::new(
            session_id.to_string(),
            sleep_id.clone(),
        ))
        .await?;
    release_runtime_terminal(session_id, requester, sleep_id).await
}

async fn release_runtime_terminal<C: RuntimeToolRequester + Sync>(
    session_id: &str,
    requester: &C,
    terminal_id: String,
) -> Result<(), acp::Error> {
    requester
        .release_terminal(schema::ReleaseTerminalRequest::new(
            session_id.to_string(),
            terminal_id,
        ))
        .await?;
    Ok(())
}

fn permission_options() -> Vec<schema::PermissionOption> {
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
    ]
}

fn runtime_tools_reply(summary: RuntimeToolSummary) -> String {
    format!(
        "Runtime tools verified: read `/workspace/README.md` => `{}`; wrote `/workspace/acp-mock-runtime-tools.txt`; terminal output `{}` with exit code {}; killed a long-running terminal.",
        summary.read_content.trim(),
        summary.terminal_output.trim(),
        summary
            .terminal_exit_code
            .map(|code| code.to_string())
            .unwrap_or_else(|| "unknown".to_string()),
    )
}

fn runtime_tools_unavailable_reply() -> String {
    "Runtime tools are unavailable because the connected client did not advertise fs/read_text_file, fs/write_text_file, and terminal capabilities. Start a runtime-tool-capable session or run the acp-mock runtime E2E test to verify this path.".to_string()
}

fn startup_hint_message(supports_runtime_tools: bool) -> String {
    if supports_runtime_tools {
        format!(
            "Bundled mock ready.\nTry `{MANUAL_PERMISSION_TRIGGER}` for a permission request, `{MANUAL_RUNTIME_TOOLS_TRIGGER}` for runtime fs/terminal tools, or `{MANUAL_CANCEL_TRIGGER}` to test cancellation."
        )
    } else {
        format!(
            "Bundled mock ready.\nTry `{MANUAL_PERMISSION_TRIGGER}` for a permission request or `{MANUAL_CANCEL_TRIGGER}` to test cancellation."
        )
    }
}

fn add_auth_methods(
    response: schema::InitializeResponse,
    auth_required: bool,
) -> schema::InitializeResponse {
    if auth_required {
        response.auth_methods(vec![schema::AuthMethod::Agent(
            schema::AuthMethodAgent::new(MOCK_AUTH_METHOD_ID, "Mock agent auth"),
        )])
    } else {
        response
    }
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

fn build_mock_agent_connector(agent: MockAgent) -> acp::DynConnectTo<acp::Client> {
    acp::DynConnectTo::new(
        acp::Builder::new_with(acp::Agent, MockDispatchHandler::new(agent)).name("acp-mock"),
    )
}

type DynRd = Box<dyn AsyncRead + Send + Unpin>;
type DynWr = Box<dyn AsyncWrite + Send + Unpin>;
type LineSink = Pin<Box<dyn Sink<String, Error = std::io::Error> + Send>>;
type LineStream = Pin<Box<dyn Stream<Item = std::io::Result<String>> + Send>>;

struct MockIo {
    outgoing: DynWr,
    incoming: DynRd,
}

impl MockIo {
    fn new(reader: OwnedReadHalf, writer: OwnedWriteHalf) -> Self {
        Self {
            outgoing: Box::new(writer.compat_write()) as DynWr,
            incoming: Box::new(reader.compat()) as DynRd,
        }
    }
}

async fn write_mock_io_line(mut writer: DynWr, line: String) -> std::io::Result<DynWr> {
    let mut bytes = line.into_bytes();
    bytes.push(b'\n');
    writer.write_all(&bytes).await?;
    Ok(writer)
}

impl<R: acp::Role> ConnectTo<R> for MockIo {
    async fn connect_to(self, client: impl ConnectTo<R::Counterpart>) -> Result<(), acp::Error> {
        let (channel, serve_io) = <MockIo as ConnectTo<R>>::into_channel_and_future(self);
        let client = acp::DynConnectTo::new(client);
        let serve_client: BoxFuture<'static, acp::Result<()>> =
            Box::pin(client.connect_to(channel));
        match future::select(serve_client, serve_io).await {
            Either::Left((result, _)) | Either::Right((result, _)) => result,
        }
    }

    fn into_channel_and_future(self) -> (acp::Channel, BoxFuture<'static, acp::Result<()>>) {
        let Self { outgoing, incoming } = self;
        let incoming_lines: LineStream = Box::pin(BufReader::new(incoming).lines());
        let outgoing_sink: LineSink = Box::pin(unfold(outgoing, write_mock_io_line));
        ConnectTo::<R>::into_channel_and_future(acp::Lines::new(outgoing_sink, incoming_lines))
    }
}

#[rustfmt::skip]
async fn connect_mock_agent(reader: OwnedReadHalf, writer: OwnedWriteHalf, agent: MockAgent) -> Result<(), acp::Error> { ConnectTo::<acp::Agent>::connect_to(MockIo::new(reader, writer), build_mock_agent_connector(agent)).await }

#[rustfmt::skip]
async fn handle_connection(stream: TcpStream, state: Arc<MockServerState>) -> Result<(), acp::Error> { let (reader, writer) = stream.into_split(); connect_mock_agent(reader, writer, MockAgent::new(state)).await }

#[cfg(test)]
mod coverage_tests {
    use std::{net::SocketAddr, sync::Arc, time::Duration};

    use agent_client_protocol::{ConnectTo, HandleDispatchFrom, JsonRpcMessage, schema};
    use tokio::{
        io::{duplex, split},
        net::{TcpListener, TcpStream},
        task::JoinHandle,
    };
    use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

    use super::{
        MANUAL_PERMISSION_TRIGGER, MockAgent, MockConfig, MockDispatchHandler, MockIo,
        MockServerState, connect_mock_agent, prompt_response_for_permission_outcome,
        spawn_with_shutdown_task,
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
                auth_required: false,
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
    async fn accept_mock_agent_connection(listener: TcpListener, state: Arc<MockServerState>) -> Result<(), acp::Error> { let (stream, _) = listener.accept().await.expect("test listener should accept"); let (reader, writer) = stream.into_split(); connect_mock_agent(reader, writer, MockAgent::new(state)).await }

    #[tokio::test(flavor = "current_thread")]
    async fn connect_mock_agent_accepts_direct_acp_roundtrips() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test listener should bind");
        let address = listener
            .local_addr()
            .expect("test listener should expose a local address");
        let state = Arc::new(MockServerState::new(MockConfig {
            response_delay: Duration::from_millis(1),
            startup_hints: false,
            auth_required: false,
        }));
        let server = tokio::spawn(accept_mock_agent_connection(listener, state));

        let stream = connect_roundtrip_stream(address).await;
        let (reader, writer) = stream.into_split();
        let working_dir = std::env::current_dir().expect("current directory should be available");
        let result = acp::Client
            .builder()
            .name("acp-mock-connect-agent-test")
            .connect_with(
                acp::ByteStreams::new(writer.compat_write(), reader.compat()),
                move |connection: acp::ConnectionTo<acp::Agent>| async move {
                    initialize_roundtrip_connection(&connection).await?;
                    connection
                        .send_request(schema::LoadSessionRequest::new("mock_0", working_dir))
                        .block_task()
                        .await
                },
            )
            .await
            .expect("direct ACP roundtrip should succeed");

        assert_eq!(result, schema::LoadSessionResponse::new());
        server.abort();
        let _ = server.await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn mock_io_connect_to_completes_when_peer_closes() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test listener should bind");
        let address = listener
            .local_addr()
            .expect("test listener should expose a local address");
        let (client_stream, accepted) =
            tokio::join!(TcpStream::connect(address), listener.accept());
        let client_stream = client_stream.expect("test client should connect");
        let (server_stream, _) = accepted.expect("test listener should accept");
        let (reader, writer) = server_stream.into_split();
        let (channel, counterpart) = acp::Channel::duplex();

        drop(channel);
        drop(client_stream);

        let _ = tokio::time::timeout(
            Duration::from_secs(1),
            ConnectTo::<acp::Agent>::connect_to(MockIo::new(reader, writer), counterpart),
        )
        .await
        .expect("MockIo should stop promptly when the peer closes");
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
                auth_required: false,
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
