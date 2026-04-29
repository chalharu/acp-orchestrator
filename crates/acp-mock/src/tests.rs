use super::*;
use crate::prompt::{
    MANUAL_CANCEL_TRIGGER, MANUAL_FAILURE_TRIGGER, MANUAL_PERMISSION_TRIGGER,
    prompt_requires_permission, prompt_should_fail, reply_for, response_delay_for, wait_for_cancel,
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
        startup_hints: false,
    })))
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
                assert!(text.text.contains(MANUAL_CANCEL_TRIGGER));
            }
            other => panic!("unexpected startup hint content: {other:?}"),
        },
        other => panic!("unexpected startup hint update: {other:?}"),
    }
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
