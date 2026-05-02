use super::*;
use crate::prompt::{
    MANUAL_CANCEL_TRIGGER, MANUAL_FAILURE_TRIGGER, MANUAL_PERMISSION_TRIGGER,
    MANUAL_RUNTIME_TOOLS_TRIGGER, prompt_requires_permission, prompt_should_fail,
    prompt_uses_runtime_tools, reply_for, response_delay_for, wait_for_cancel,
};
use agent_client_protocol::schema;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

struct StubPermissionRequester {
    call_count: Arc<AtomicUsize>,
    response: schema::RequestPermissionResponse,
}

#[async_trait::async_trait]
impl PermissionRequester for StubPermissionRequester {
    async fn request_permission(
        &self,
        _request: schema::RequestPermissionRequest,
    ) -> Result<schema::RequestPermissionResponse, acp::Error> {
        self.call_count.fetch_add(1, Ordering::Relaxed);
        Ok(self.response.clone())
    }
}

#[async_trait::async_trait]
impl RuntimeToolRequester for StubPermissionRequester {
    async fn read_text_file(
        &self,
        _request: schema::ReadTextFileRequest,
    ) -> Result<schema::ReadTextFileResponse, acp::Error> {
        Err(acp::Error::internal_error())
    }

    async fn write_text_file(
        &self,
        _request: schema::WriteTextFileRequest,
    ) -> Result<schema::WriteTextFileResponse, acp::Error> {
        Err(acp::Error::internal_error())
    }

    async fn create_terminal(
        &self,
        _request: schema::CreateTerminalRequest,
    ) -> Result<schema::CreateTerminalResponse, acp::Error> {
        Err(acp::Error::internal_error())
    }

    async fn terminal_output(
        &self,
        _request: schema::TerminalOutputRequest,
    ) -> Result<schema::TerminalOutputResponse, acp::Error> {
        Err(acp::Error::internal_error())
    }

    async fn wait_for_terminal_exit(
        &self,
        _request: schema::WaitForTerminalExitRequest,
    ) -> Result<schema::WaitForTerminalExitResponse, acp::Error> {
        Err(acp::Error::internal_error())
    }

    async fn kill_terminal(
        &self,
        _request: schema::KillTerminalRequest,
    ) -> Result<schema::KillTerminalResponse, acp::Error> {
        Err(acp::Error::internal_error())
    }

    async fn release_terminal(
        &self,
        _request: schema::ReleaseTerminalRequest,
    ) -> Result<schema::ReleaseTerminalResponse, acp::Error> {
        Err(acp::Error::internal_error())
    }
}

struct StubRuntimeRequester {
    permission_count: Arc<AtomicUsize>,
    calls: Arc<Mutex<Vec<&'static str>>>,
    permission_response: schema::RequestPermissionResponse,
    fail_on: Option<&'static str>,
}

impl StubRuntimeRequester {
    fn new() -> Self {
        Self {
            permission_count: Arc::new(AtomicUsize::new(0)),
            calls: Arc::new(Mutex::new(Vec::new())),
            permission_response: schema::RequestPermissionResponse::new(
                schema::RequestPermissionOutcome::Selected(schema::SelectedPermissionOutcome::new(
                    "allow_once",
                )),
            ),
            fail_on: None,
        }
    }

    fn with_permission_outcome(mut self, outcome: schema::RequestPermissionOutcome) -> Self {
        self.permission_response = schema::RequestPermissionResponse::new(outcome);
        self
    }

    fn failing_on(mut self, name: &'static str) -> Self {
        self.fail_on = Some(name);
        self
    }

    fn calls(&self) -> Vec<&'static str> {
        self.calls
            .lock()
            .expect("runtime call list should not be poisoned")
            .clone()
    }

    fn push_call(&self, name: &'static str) {
        self.calls
            .lock()
            .expect("runtime call list should not be poisoned")
            .push(name);
    }

    fn fail_if_configured(&self, name: &'static str) -> Result<(), acp::Error> {
        if self.fail_on == Some(name) {
            Err(acp::Error::internal_error())
        } else {
            Ok(())
        }
    }
}

#[async_trait::async_trait]
impl PermissionRequester for StubRuntimeRequester {
    async fn request_permission(
        &self,
        request: schema::RequestPermissionRequest,
    ) -> Result<schema::RequestPermissionResponse, acp::Error> {
        self.permission_count.fetch_add(1, Ordering::Relaxed);
        assert_eq!(
            request.tool_call.fields.title.as_deref(),
            Some("verify runtime tools")
        );
        Ok(self.permission_response.clone())
    }
}

#[async_trait::async_trait]
impl RuntimeToolRequester for StubRuntimeRequester {
    async fn read_text_file(
        &self,
        request: schema::ReadTextFileRequest,
    ) -> Result<schema::ReadTextFileResponse, acp::Error> {
        self.push_call("read");
        assert_eq!(request.path, std::path::Path::new("/workspace/README.md"));
        self.fail_if_configured("read")?;
        Ok(schema::ReadTextFileResponse::new("runtime-readme"))
    }

    async fn write_text_file(
        &self,
        request: schema::WriteTextFileRequest,
    ) -> Result<schema::WriteTextFileResponse, acp::Error> {
        self.push_call("write");
        assert_eq!(
            request.path,
            std::path::Path::new("/workspace/acp-mock-runtime-tools.txt")
        );
        assert_eq!(request.content, "created by acp-mock runtime tools\n");
        self.fail_if_configured("write")?;
        Ok(schema::WriteTextFileResponse::new())
    }

    async fn create_terminal(
        &self,
        request: schema::CreateTerminalRequest,
    ) -> Result<schema::CreateTerminalResponse, acp::Error> {
        self.push_call("create_terminal");
        self.fail_if_configured("create_terminal")?;
        match request.command.as_str() {
            "/bin/printf" => Ok(schema::CreateTerminalResponse::new("printf")),
            "/bin/sleep" => Ok(schema::CreateTerminalResponse::new("sleep")),
            _ => Err(acp::Error::invalid_params()),
        }
    }

    async fn terminal_output(
        &self,
        request: schema::TerminalOutputRequest,
    ) -> Result<schema::TerminalOutputResponse, acp::Error> {
        self.push_call("terminal_output");
        assert_eq!(request.terminal_id.to_string(), "printf");
        self.fail_if_configured("terminal_output")?;
        Ok(schema::TerminalOutputResponse::new("terminal-ok", false))
    }

    async fn wait_for_terminal_exit(
        &self,
        request: schema::WaitForTerminalExitRequest,
    ) -> Result<schema::WaitForTerminalExitResponse, acp::Error> {
        self.push_call("wait_for_terminal_exit");
        assert_eq!(request.terminal_id.to_string(), "printf");
        self.fail_if_configured("wait_for_terminal_exit")?;
        Ok(schema::WaitForTerminalExitResponse::new(
            schema::TerminalExitStatus::new().exit_code(0),
        ))
    }

    async fn kill_terminal(
        &self,
        request: schema::KillTerminalRequest,
    ) -> Result<schema::KillTerminalResponse, acp::Error> {
        self.push_call("kill_terminal");
        assert_eq!(request.terminal_id.to_string(), "sleep");
        self.fail_if_configured("kill_terminal")?;
        Ok(schema::KillTerminalResponse::new())
    }

    async fn release_terminal(
        &self,
        request: schema::ReleaseTerminalRequest,
    ) -> Result<schema::ReleaseTerminalResponse, acp::Error> {
        self.push_call("release_terminal");
        assert!(
            matches!(request.terminal_id.to_string().as_str(), "printf" | "sleep"),
            "unexpected terminal id {}",
            request.terminal_id
        );
        self.fail_if_configured("release_terminal")?;
        Ok(schema::ReleaseTerminalResponse::new())
    }
}

struct StubSessionUpdateNotifier {
    should_fail: bool,
    call_count: Arc<AtomicUsize>,
    last_notification: Arc<Mutex<Option<schema::SessionNotification>>>,
}

#[async_trait::async_trait]
impl SessionUpdateNotifier for StubSessionUpdateNotifier {
    async fn send_session_update(
        &self,
        notification: schema::SessionNotification,
    ) -> Result<(), acp::Error> {
        self.call_count.fetch_add(1, Ordering::Relaxed);
        *self
            .last_notification
            .lock()
            .expect("last notification mutex should not be poisoned") = Some(notification);
        if self.should_fail {
            Err(acp::Error::internal_error())
        } else {
            Ok(())
        }
    }
}

struct RecordingSessionUpdateNotifier {
    notifications: Arc<Mutex<Vec<schema::SessionNotification>>>,
}

impl RecordingSessionUpdateNotifier {
    fn new() -> Self {
        Self {
            notifications: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn notifications(&self) -> Vec<schema::SessionNotification> {
        self.notifications
            .lock()
            .expect("notification list should not be poisoned")
            .clone()
    }
}

#[async_trait::async_trait]
impl SessionUpdateNotifier for RecordingSessionUpdateNotifier {
    async fn send_session_update(
        &self,
        notification: schema::SessionNotification,
    ) -> Result<(), acp::Error> {
        self.notifications
            .lock()
            .expect("notification list should not be poisoned")
            .push(notification);
        Ok(())
    }
}

fn allow_once_requester() -> StubPermissionRequester {
    StubPermissionRequester {
        call_count: Arc::new(AtomicUsize::new(0)),
        response: schema::RequestPermissionResponse::new(
            schema::RequestPermissionOutcome::Selected(schema::SelectedPermissionOutcome::new(
                "allow_once",
            )),
        ),
    }
}

fn quiet_notifier() -> StubSessionUpdateNotifier {
    StubSessionUpdateNotifier {
        should_fail: false,
        call_count: Arc::new(AtomicUsize::new(0)),
        last_notification: Arc::new(Mutex::new(None)),
    }
}

fn mock_agent_with_delay(delay: Duration) -> MockAgent {
    MockAgent::new(Arc::new(MockServerState::new(MockConfig {
        response_delay: delay,
        ..MockConfig::default()
    })))
}

fn auth_required_state() -> Arc<MockServerState> {
    Arc::new(MockServerState::new(MockConfig {
        auth_required: true,
        ..MockConfig::default()
    }))
}

fn assert_auth_required(error: acp::Error) {
    assert_eq!(error.code, acp::ErrorCode::AuthRequired);
}

fn spawn_cancel_task(agent: MockAgent) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(5)).await;
        agent
            .cancel(schema::CancelNotification::new("mock_0"))
            .await
            .expect("cancel notifications should succeed");
    })
}

fn runtime_capable_initialize_request() -> schema::InitializeRequest {
    schema::InitializeRequest::new(schema::ProtocolVersion::V1).client_capabilities(
        schema::ClientCapabilities::new()
            .fs(schema::FileSystemCapabilities::new()
                .read_text_file(true)
                .write_text_file(true))
            .terminal(true),
    )
}

#[tokio::test]
async fn mock_agent_supports_control_plane_requests() {
    let agent = MockAgent::new(Arc::new(MockServerState::new(MockConfig::default())));
    let notifier = StubSessionUpdateNotifier {
        should_fail: false,
        call_count: Arc::new(AtomicUsize::new(0)),
        last_notification: Arc::new(Mutex::new(None)),
    };

    let authenticate = agent
        .authenticate(schema::AuthenticateRequest::new("local"))
        .await
        .expect("authenticate requests should succeed");
    assert_eq!(authenticate, schema::AuthenticateResponse::default());

    let session = agent
        .new_session(schema::NewSessionRequest::new("/tmp"), &notifier)
        .await
        .expect("new sessions should succeed");
    assert_eq!(session.session_id.to_string(), "mock_0");

    let loaded = agent
        .load_session(schema::LoadSessionRequest::new(
            session.session_id.clone(),
            "/tmp",
        ))
        .await
        .expect("load session requests should succeed");
    assert_eq!(loaded, schema::LoadSessionResponse::new());

    agent
        .cancel(schema::CancelNotification::new(session.session_id.clone()))
        .await
        .expect("cancel notifications should succeed");

    let mode = agent
        .set_session_mode(schema::SetSessionModeRequest::new(
            session.session_id,
            "default",
        ))
        .await
        .expect("set session mode requests should succeed");
    assert_eq!(mode, schema::SetSessionModeResponse::default());
}

#[tokio::test]
async fn cancel_notifications_mark_subscribed_sessions_as_cancelled() {
    let agent = MockAgent::new(Arc::new(MockServerState::new(MockConfig::default())));
    let session = agent.state.session_state("mock_0");
    let (mut cancel_rx, generation) = session.subscribe_cancel();

    handle_cancel_notification(agent, schema::CancelNotification::new("mock_0"))
        .await
        .expect("cancel notifications should succeed");

    assert!(wait_for_cancel(&mut cancel_rx, generation, Duration::from_secs(1)).await);
}

#[tokio::test]
async fn mock_agent_initializes_with_mock_identity() {
    let agent = MockAgent::new(Arc::new(MockServerState::new(MockConfig::default())));

    let response = agent
        .initialize(schema::InitializeRequest::new(schema::ProtocolVersion::V1))
        .await
        .expect("initialize requests should succeed");
    let debug = format!("{response:?}");

    assert!(debug.contains("acp-mock"));
    assert!(debug.contains("ACP Mock"));
    assert!(response.agent_capabilities.load_session);
    assert!(!agent.supports_runtime_tools());
}

#[tokio::test]
async fn mock_agent_records_runtime_tool_client_capabilities() {
    let agent = MockAgent::new(Arc::new(MockServerState::new(MockConfig::default())));

    agent
        .initialize(runtime_capable_initialize_request())
        .await
        .expect("initialize requests should succeed");

    assert!(agent.supports_runtime_tools());
}

#[tokio::test]
async fn mock_agent_advertises_and_enforces_authentication_when_required() {
    let agent = MockAgent::new(auth_required_state());
    let response = agent
        .initialize(schema::InitializeRequest::new(schema::ProtocolVersion::V1))
        .await
        .expect("initialize requests should succeed");
    let unauthenticated = agent
        .new_session(schema::NewSessionRequest::new("/tmp"), &quiet_notifier())
        .await
        .expect_err("new sessions should require authentication");

    assert_eq!(response.auth_methods.len(), 1);
    assert_auth_required(unauthenticated);
    agent
        .authenticate(schema::AuthenticateRequest::new(
            response.auth_methods[0].id().clone(),
        ))
        .await
        .expect("advertised auth methods should authenticate");
    agent
        .new_session(schema::NewSessionRequest::new("/tmp"), &quiet_notifier())
        .await
        .expect("authenticated new sessions should succeed");
}

#[tokio::test]
async fn mock_agent_rejects_unknown_auth_method_when_authentication_is_required() {
    let agent = MockAgent::new(auth_required_state());

    let error = agent
        .authenticate(schema::AuthenticateRequest::new("unknown-auth-method"))
        .await
        .expect_err("unknown auth methods should be rejected");

    assert_eq!(error.message, acp::Error::invalid_params().message);
}

#[tokio::test]
async fn mock_agent_authentication_is_connection_scoped() {
    let state = auth_required_state();
    let authenticated = MockAgent::new(state.clone());
    authenticated
        .authenticate(schema::AuthenticateRequest::new(MOCK_AUTH_METHOD_ID))
        .await
        .expect("first connection should authenticate");
    let fresh_connection = MockAgent::new(state);

    let new_session = fresh_connection
        .new_session(schema::NewSessionRequest::new("/tmp"), &quiet_notifier())
        .await
        .expect_err("fresh connections should not inherit auth state");
    let load_session = fresh_connection
        .load_session(schema::LoadSessionRequest::new("mock_0", "/tmp"))
        .await
        .expect_err("load session should require authentication");
    let prompt = fresh_connection
        .prompt(
            schema::PromptRequest::new("mock_0", vec!["hello".to_string().into()]),
            &quiet_notifier(),
            &allow_once_requester(),
        )
        .await
        .expect_err("prompts should require authentication");

    assert_auth_required(new_session);
    assert_auth_required(load_session);
    assert_auth_required(prompt);
}

#[tokio::test]
async fn mock_agent_emits_startup_hints_when_enabled() {
    let notifier = StubSessionUpdateNotifier {
        should_fail: false,
        call_count: Arc::new(AtomicUsize::new(0)),
        last_notification: Arc::new(Mutex::new(None)),
    };
    let agent = MockAgent::new(Arc::new(MockServerState::new(MockConfig {
        response_delay: Duration::from_millis(120),
        startup_hints: true,
        ..MockConfig::default()
    })));

    agent
        .new_session(schema::NewSessionRequest::new("/tmp"), &notifier)
        .await
        .expect("new sessions should succeed");

    let notification = notifier
        .last_notification
        .lock()
        .expect("last notification mutex should not be poisoned")
        .clone()
        .expect("startup hints should be sent");
    match notification.update {
        schema::SessionUpdate::AgentMessageChunk(chunk) => match chunk.content {
            schema::ContentBlock::Text(text) => {
                assert!(text.text.contains(MANUAL_PERMISSION_TRIGGER));
                assert!(!text.text.contains(MANUAL_RUNTIME_TOOLS_TRIGGER));
                assert!(text.text.contains(MANUAL_CANCEL_TRIGGER));
            }
            other => panic!("unexpected startup hint content: {other:?}"),
        },
        other => panic!("unexpected startup hint update: {other:?}"),
    }
}

#[tokio::test]
async fn mock_agent_emits_runtime_tool_startup_hint_when_client_supports_tools() {
    let notifier = StubSessionUpdateNotifier {
        should_fail: false,
        call_count: Arc::new(AtomicUsize::new(0)),
        last_notification: Arc::new(Mutex::new(None)),
    };
    let agent = MockAgent::new(Arc::new(MockServerState::new(MockConfig {
        startup_hints: true,
        ..MockConfig::default()
    })));
    agent
        .initialize(runtime_capable_initialize_request())
        .await
        .expect("initialize requests should succeed");

    agent
        .new_session(schema::NewSessionRequest::new("/tmp"), &notifier)
        .await
        .expect("new sessions should succeed");

    let notification = notifier
        .last_notification
        .lock()
        .expect("last notification mutex should not be poisoned")
        .clone()
        .expect("startup hints should be sent");
    assert!(format!("{notification:?}").contains(MANUAL_RUNTIME_TOOLS_TRIGGER));
}

#[tokio::test]
async fn mock_agent_runtime_tool_trigger_exercises_client_tools() {
    let RuntimePromptRun {
        requester,
        result,
        notifications,
    } = run_runtime_tool_prompt(StubRuntimeRequester::new()).await;
    let response = result.expect("runtime tool prompt should resolve");

    assert_eq!(
        response,
        schema::PromptResponse::new(schema::StopReason::EndTurn)
    );
    assert_eq!(requester.permission_count.load(Ordering::Relaxed), 1);
    assert_runtime_request_sequence(&requester);
    assert_runtime_notifications_completed(&notifications);
}

#[tokio::test]
async fn mock_agent_runtime_tool_trigger_reports_unavailable_without_client_capabilities() {
    let RuntimePromptRun {
        requester,
        result,
        notifications,
    } = run_runtime_tool_prompt_with_capabilities(StubRuntimeRequester::new(), false).await;

    assert_eq!(
        result.expect("unsupported runtime tool prompt should resolve"),
        schema::PromptResponse::new(schema::StopReason::EndTurn)
    );
    assert_eq!(requester.permission_count.load(Ordering::Relaxed), 0);
    assert_eq!(requester.calls(), Vec::<&'static str>::new());
    assert!(matches!(
        notifications[0].update,
        schema::SessionUpdate::AgentMessageChunk(ref chunk)
            if format!("{:?}", chunk.content).contains("Runtime tools are unavailable")
    ));
}

#[tokio::test]
async fn mock_agent_runtime_tool_trigger_marks_failed_when_permission_rejected() {
    let requester = StubRuntimeRequester::new().with_permission_outcome(
        schema::RequestPermissionOutcome::Selected(schema::SelectedPermissionOutcome::new(
            "reject_once",
        )),
    );
    let RuntimePromptRun {
        requester,
        result,
        notifications,
    } = run_runtime_tool_prompt(requester).await;

    assert_eq!(
        result.expect("permission rejection should resolve the prompt"),
        schema::PromptResponse::new(schema::StopReason::EndTurn)
    );
    assert_eq!(requester.calls(), Vec::<&'static str>::new());
    assert_runtime_notifications_failed(&notifications);
}

#[tokio::test]
async fn mock_agent_runtime_tool_trigger_marks_failed_when_client_request_errors() {
    let RuntimePromptRun {
        requester,
        result,
        notifications,
    } = run_runtime_tool_prompt(StubRuntimeRequester::new().failing_on("read")).await;

    assert!(result.is_err());
    assert_eq!(requester.calls(), vec!["read"]);
    assert_runtime_notifications_failed(&notifications);
}

struct RuntimePromptRun {
    requester: StubRuntimeRequester,
    result: Result<schema::PromptResponse, acp::Error>,
    notifications: Vec<schema::SessionNotification>,
}

async fn run_runtime_tool_prompt(requester: StubRuntimeRequester) -> RuntimePromptRun {
    run_runtime_tool_prompt_with_capabilities(requester, true).await
}

async fn run_runtime_tool_prompt_with_capabilities(
    requester: StubRuntimeRequester,
    supports_runtime_tools: bool,
) -> RuntimePromptRun {
    let notifier = RecordingSessionUpdateNotifier::new();
    let agent = MockAgent::new(Arc::new(MockServerState::new(MockConfig::default())));
    if supports_runtime_tools {
        agent
            .initialize(runtime_capable_initialize_request())
            .await
            .expect("initialize requests should succeed");
    }

    let result = agent
        .prompt(
            schema::PromptRequest::new(
                "mock_0",
                vec![schema::ContentBlock::Text(schema::TextContent::new(
                    MANUAL_RUNTIME_TOOLS_TRIGGER,
                ))],
            ),
            &notifier,
            &requester,
        )
        .await;

    RuntimePromptRun {
        requester,
        result,
        notifications: notifier.notifications(),
    }
}

fn assert_runtime_request_sequence(requester: &StubRuntimeRequester) {
    assert_eq!(
        requester.calls().as_slice(),
        vec![
            "read",
            "write",
            "create_terminal",
            "wait_for_terminal_exit",
            "terminal_output",
            "release_terminal",
            "create_terminal",
            "kill_terminal",
            "release_terminal",
        ]
        .as_slice()
    );
}

fn assert_runtime_tool_started(notifications: &[schema::SessionNotification]) {
    assert!(matches!(
        notifications[0].update,
        schema::SessionUpdate::ToolCall(ref call)
            if call.title == "Verify ACP runtime tools"
                && call.status == schema::ToolCallStatus::InProgress
    ));
}

fn assert_runtime_notifications_completed(notifications: &[schema::SessionNotification]) {
    assert_runtime_tool_started(notifications);
    assert!(matches!(
        notifications[1].update,
        schema::SessionUpdate::ToolCallUpdate(ref update)
            if update.fields.status == Some(schema::ToolCallStatus::Completed)
    ));
    assert!(matches!(
        notifications[2].update,
        schema::SessionUpdate::AgentMessageChunk(ref chunk)
            if format!("{:?}", chunk.content).contains("Runtime tools verified")
    ));
}

fn assert_runtime_notifications_failed(notifications: &[schema::SessionNotification]) {
    assert_runtime_tool_started(notifications);
    assert!(matches!(
        notifications[1].update,
        schema::SessionUpdate::ToolCallUpdate(ref update)
            if update.fields.status == Some(schema::ToolCallStatus::Failed)
    ));
}

#[tokio::test]
async fn mock_agent_permission_requests_include_expected_options() {
    let call_count = Arc::new(AtomicUsize::new(0));
    let requester = StubPermissionRequester {
        call_count: call_count.clone(),
        response: schema::RequestPermissionResponse::new(
            schema::RequestPermissionOutcome::Selected(schema::SelectedPermissionOutcome::new(
                "allow_once",
            )),
        ),
    };
    let notifier = StubSessionUpdateNotifier {
        should_fail: false,
        call_count: Arc::new(AtomicUsize::new(0)),
        last_notification: Arc::new(Mutex::new(None)),
    };
    let agent = MockAgent::new(Arc::new(MockServerState::new(MockConfig::default())));

    let response = agent
        .prompt(
            schema::PromptRequest::new(
                "mock_0",
                vec![schema::ContentBlock::Text(schema::TextContent::new(
                    MANUAL_PERMISSION_TRIGGER,
                ))],
            ),
            &notifier,
            &requester,
        )
        .await
        .expect("permission requests should resolve");

    assert_eq!(call_count.load(Ordering::Relaxed), 1);
    assert_eq!(
        response,
        schema::PromptResponse::new(schema::StopReason::EndTurn)
    );
}

#[tokio::test]
async fn mock_agent_cancels_prompts_when_permissions_are_cancelled() {
    let requester = StubPermissionRequester {
        call_count: Arc::new(AtomicUsize::new(0)),
        response: schema::RequestPermissionResponse::new(
            schema::RequestPermissionOutcome::Cancelled,
        ),
    };
    let notifier = StubSessionUpdateNotifier {
        should_fail: false,
        call_count: Arc::new(AtomicUsize::new(0)),
        last_notification: Arc::new(Mutex::new(None)),
    };
    let agent = MockAgent::new(Arc::new(MockServerState::new(MockConfig::default())));

    let response = agent
        .prompt(
            schema::PromptRequest::new(
                "mock_0",
                vec![schema::ContentBlock::Text(schema::TextContent::new(
                    MANUAL_PERMISSION_TRIGGER,
                ))],
            ),
            &notifier,
            &requester,
        )
        .await
        .expect("cancelled permission requests should resolve");

    assert_eq!(
        response,
        schema::PromptResponse::new(schema::StopReason::Cancelled)
    );
    assert_eq!(
        notifier.call_count.load(Ordering::Relaxed),
        0,
        "cancelled permission requests should not emit reply chunks"
    );
}

#[tokio::test]
async fn mock_agent_cancels_inflight_prompts_after_cancel_notifications() {
    let requester = allow_once_requester();
    let notifier = quiet_notifier();
    let agent = mock_agent_with_delay(Duration::from_millis(50));
    let cancel_task = spawn_cancel_task(agent.clone());

    let response = agent
        .prompt(
            schema::PromptRequest::new(
                "mock_0",
                vec![schema::ContentBlock::Text(schema::TextContent::new(
                    "hello",
                ))],
            ),
            &notifier,
            &requester,
        )
        .await
        .expect("cancelled prompts should resolve");

    cancel_task
        .await
        .expect("cancel notifications should finish");
    assert_eq!(
        response,
        schema::PromptResponse::new(schema::StopReason::Cancelled)
    );
    assert_eq!(
        notifier.call_count.load(Ordering::Relaxed),
        0,
        "cancelled prompts should not emit reply chunks"
    );
}

#[tokio::test]
async fn mock_agent_propagates_notifier_failures_while_streaming_replies() {
    let requester = StubPermissionRequester {
        call_count: Arc::new(AtomicUsize::new(0)),
        response: schema::RequestPermissionResponse::new(
            schema::RequestPermissionOutcome::Selected(schema::SelectedPermissionOutcome::new(
                "allow_once",
            )),
        ),
    };
    let notifier = StubSessionUpdateNotifier {
        should_fail: true,
        call_count: Arc::new(AtomicUsize::new(0)),
        last_notification: Arc::new(Mutex::new(None)),
    };
    let agent = MockAgent::new(Arc::new(MockServerState::new(MockConfig::default())));

    let error = agent
        .prompt(
            schema::PromptRequest::new(
                "mock_0",
                vec![schema::ContentBlock::Text(schema::TextContent::new(
                    "hello",
                ))],
            ),
            &notifier,
            &requester,
        )
        .await
        .expect_err("notifier failures should fail the prompt");

    assert_eq!(error.message, acp::Error::internal_error().message);
    assert_eq!(notifier.call_count.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn mock_agent_rejects_permission_denials_without_replying() {
    let requester = StubPermissionRequester {
        call_count: Arc::new(AtomicUsize::new(0)),
        response: schema::RequestPermissionResponse::new(
            schema::RequestPermissionOutcome::Selected(schema::SelectedPermissionOutcome::new(
                "reject_once",
            )),
        ),
    };
    let notifier = StubSessionUpdateNotifier {
        should_fail: false,
        call_count: Arc::new(AtomicUsize::new(0)),
        last_notification: Arc::new(Mutex::new(None)),
    };
    let agent = MockAgent::new(Arc::new(MockServerState::new(MockConfig::default())));

    let response = agent
        .prompt(
            schema::PromptRequest::new(
                "mock_0",
                vec![schema::ContentBlock::Text(schema::TextContent::new(
                    MANUAL_PERMISSION_TRIGGER,
                ))],
            ),
            &notifier,
            &requester,
        )
        .await
        .expect("permission requests should resolve");

    assert_eq!(
        response,
        schema::PromptResponse::new(schema::StopReason::EndTurn)
    );
    assert_eq!(
        notifier.call_count.load(Ordering::Relaxed),
        0,
        "denied permissions should not emit reply chunks"
    );
}

#[test]
fn permission_request_uses_once_options() {
    let request = permission_request("mock_0".to_string(), "tool_0".to_string());

    assert_eq!(request.session_id.to_string(), "mock_0");
    assert_eq!(request.tool_call.tool_call_id.to_string(), "tool_0");
    assert_eq!(
        request.tool_call.fields.title.as_deref(),
        Some("read_text_file README.md")
    );
    assert_eq!(request.options.len(), 2);
    assert_eq!(request.options[0].option_id.to_string(), "allow_once");
    assert_eq!(request.options[1].option_id.to_string(), "reject_once");
}

#[test]
fn default_config_uses_the_expected_delay() {
    assert_eq!(
        MockConfig::default().response_delay,
        Duration::from_millis(120)
    );
    assert!(!MockConfig::default().startup_hints);
    assert!(!MockConfig::default().auth_required);
}

#[test]
fn prompt_text_formats_binary_placeholders_and_resource_links() {
    let prompt = vec![
        schema::ContentBlock::Image(schema::ImageContent::new("aGVsbG8=", "image/png")),
        schema::ContentBlock::Audio(schema::AudioContent::new("aGVsbG8=", "audio/wav")),
        schema::ContentBlock::ResourceLink(schema::ResourceLink::new("guide", "file:///guide.md")),
        schema::ContentBlock::Resource(schema::EmbeddedResource::new(
            schema::EmbeddedResourceResource::TextResourceContents(
                schema::TextResourceContents::new("hello", "file:///embedded.md"),
            ),
        )),
    ];

    assert_eq!(
        prompt_text(&prompt),
        "<image> <audio> file:///guide.md <resource>"
    );
}

#[test]
fn long_prompts_are_truncated_in_mock_replies() {
    let prompt = "word ".repeat(80);
    let reply = reply_for(&prompt);

    assert!(reply.contains("...`"));
    assert!(reply.starts_with("mock assistant: I received `"));
    assert!(reply.contains("ACP round-trip succeeded"));
}

#[test]
fn manual_permission_trigger_still_uses_the_permission_flow() {
    assert!(prompt_requires_permission(MANUAL_PERMISSION_TRIGGER));
}

#[test]
fn manual_runtime_tools_trigger_uses_the_runtime_tool_flow() {
    assert!(prompt_uses_runtime_tools(MANUAL_RUNTIME_TOOLS_TRIGGER));
}

#[test]
fn manual_permission_trigger_is_case_and_spacing_insensitive() {
    assert!(prompt_requires_permission("  Verify   Permission "));
}

#[test]
fn bare_permission_words_do_not_trigger_the_permission_flow() {
    assert!(!prompt_requires_permission("permission"));
}

#[test]
fn manual_failure_trigger_uses_the_failure_flow() {
    assert!(prompt_should_fail(MANUAL_FAILURE_TRIGGER));
}

#[test]
fn manual_failure_trigger_is_case_and_spacing_insensitive() {
    assert!(prompt_should_fail("  This   Will   Fail "));
}

#[test]
fn ordinary_prompts_do_not_trigger_failure() {
    assert!(!prompt_should_fail("hello"));
}

#[test]
fn manual_cancel_trigger_uses_an_extended_delay() {
    assert_eq!(
        response_delay_for(MANUAL_CANCEL_TRIGGER, Duration::from_millis(120)),
        Duration::from_secs(10)
    );
}

#[test]
fn ordinary_prompts_keep_the_default_delay() {
    let default_delay = Duration::from_millis(120);

    assert_eq!(response_delay_for("hello", default_delay), default_delay);
}

#[tokio::test]
async fn wait_for_cancel_returns_true_when_the_generation_has_advanced() {
    let session = MockSessionState::new();
    let (mut cancel_rx, generation) = session.subscribe_cancel();
    session.cancel();

    assert!(wait_for_cancel(&mut cancel_rx, generation, Duration::from_secs(1)).await);
}

#[test]
fn logging_connection_errors_does_not_panic() {
    log_connection_result(Err(acp::Error::internal_error()));
}
