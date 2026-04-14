use super::*;
use crate::{
    notifications::{finalize_permission_request, finalize_session_update},
    prompt::reply_for,
};
use agent_client_protocol::Agent as _;
use std::rc::Rc;

struct StubPermissionRequester {
    call_count: Rc<Cell<usize>>,
}

#[async_trait::async_trait(?Send)]
impl PermissionRequester for StubPermissionRequester {
    async fn request_permission(
        &self,
        _request: acp::RequestPermissionRequest,
    ) -> Result<acp::RequestPermissionResponse, acp::Error> {
        self.call_count.set(self.call_count.get() + 1);
        Ok(acp::RequestPermissionResponse::new(
            acp::RequestPermissionOutcome::Cancelled,
        ))
    }
}

struct StubSessionUpdateNotifier {
    should_fail: bool,
    call_count: Rc<Cell<usize>>,
}

#[async_trait::async_trait(?Send)]
impl SessionUpdateNotifier for StubSessionUpdateNotifier {
    async fn send_session_update(
        &self,
        _notification: acp::SessionNotification,
    ) -> Result<(), acp::Error> {
        self.call_count.set(self.call_count.get() + 1);
        if self.should_fail {
            Err(acp::Error::internal_error())
        } else {
            Ok(())
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn mock_agent_supports_control_plane_requests() {
    let state = Rc::new(MockServerState::new(MockConfig::default()));
    let (session_update_tx, _session_update_rx) = mpsc::unbounded_channel();
    let (permission_request_tx, _permission_request_rx) = mpsc::unbounded_channel();
    let agent = MockAgent::new(state, session_update_tx, permission_request_tx);

    let authenticate = agent
        .authenticate(acp::AuthenticateRequest::new("local"))
        .await
        .expect("authenticate requests should succeed");
    assert_eq!(authenticate, acp::AuthenticateResponse::default());

    let session = agent
        .new_session(acp::NewSessionRequest::new("/tmp"))
        .await
        .expect("new sessions should succeed");
    assert_eq!(session.session_id.to_string(), "mock_0");

    let loaded = agent
        .load_session(acp::LoadSessionRequest::new(
            session.session_id.clone(),
            "/tmp",
        ))
        .await
        .expect("load session requests should succeed");
    assert_eq!(loaded, acp::LoadSessionResponse::new());

    agent
        .cancel(acp::CancelNotification::new(session.session_id.clone()))
        .await
        .expect("cancel notifications should succeed");

    let mode = agent
        .set_session_mode(acp::SetSessionModeRequest::new(
            session.session_id,
            "default",
        ))
        .await
        .expect("set session mode requests should succeed");
    assert_eq!(mode, acp::SetSessionModeResponse::default());
}

#[test]
fn finalizing_permission_requests_acknowledges_successful_notifications() {
    let (ack_tx, ack_rx) = oneshot::channel();

    assert!(finalize_permission_request(
        Ok(acp::RequestPermissionResponse::new(
            acp::RequestPermissionOutcome::Cancelled,
        )),
        ack_tx,
    ));
    let response = ack_rx
        .blocking_recv()
        .expect("permission responses should be forwarded")
        .expect("forwarded responses should stay successful");
    assert!(matches!(
        response.outcome,
        acp::RequestPermissionOutcome::Cancelled
    ));
}

#[test]
fn finalizing_permission_requests_stop_after_receiver_drop() {
    let (ack_tx, ack_rx) = oneshot::channel();
    drop(ack_rx);

    assert!(!finalize_permission_request(
        Ok(acp::RequestPermissionResponse::new(
            acp::RequestPermissionOutcome::Cancelled,
        )),
        ack_tx,
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn draining_permission_requests_returns_after_delivery_errors() {
    let call_count = Rc::new(Cell::new(0));
    let requester = StubPermissionRequester {
        call_count: call_count.clone(),
    };
    let (permission_request_tx, permission_request_rx) = mpsc::unbounded_channel();
    let (first_ack_tx, first_ack_rx) = oneshot::channel();
    let (second_ack_tx, second_ack_rx) = oneshot::channel();
    drop(first_ack_rx);
    permission_request_tx
        .send((
            acp::RequestPermissionRequest::new(
                "mock_0",
                acp::ToolCallUpdate::new("tool_0", acp::ToolCallUpdateFields::new()),
                vec![],
            ),
            first_ack_tx,
        ))
        .expect("first permission request should queue");
    permission_request_tx
        .send((
            acp::RequestPermissionRequest::new(
                "mock_0",
                acp::ToolCallUpdate::new("tool_1", acp::ToolCallUpdateFields::new()),
                vec![],
            ),
            second_ack_tx,
        ))
        .expect("second permission request should queue");
    drop(permission_request_tx);

    drain_permission_requests(&requester, permission_request_rx).await;

    assert_eq!(call_count.get(), 1);
    assert!(second_ack_rx.await.is_err());
}

#[tokio::test(flavor = "current_thread")]
async fn mock_agent_permission_requests_include_expected_options() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let state = Rc::new(MockServerState::new(MockConfig::default()));
            let (session_update_tx, _session_update_rx) = mpsc::unbounded_channel();
            let (permission_request_tx, mut permission_request_rx) = mpsc::unbounded_channel();
            let agent = MockAgent::new(state, session_update_tx, permission_request_tx);

            let request_task = tokio::task::spawn_local(async move {
                agent
                    .request_permission("mock_0".to_string())
                    .await
                    .expect("permission requests should resolve")
            });

            let (request, ack_tx) = permission_request_rx
                .recv()
                .await
                .expect("permission requests should be queued");
            assert_eq!(request.session_id.to_string(), "mock_0");
            assert_eq!(request.tool_call.tool_call_id.to_string(), "tool_0");
            assert_eq!(
                request.tool_call.fields.title.as_deref(),
                Some("read_text_file README.md")
            );
            assert_eq!(request.options.len(), 2);
            assert_eq!(request.options[0].option_id.to_string(), "allow_once");
            assert_eq!(request.options[1].option_id.to_string(), "reject_once");

            ack_tx
                .send(Ok(acp::RequestPermissionResponse::new(
                    acp::RequestPermissionOutcome::Selected(acp::SelectedPermissionOutcome::new(
                        "allow_once",
                    )),
                )))
                .expect("permission request outcomes should be delivered");

            let response = request_task
                .await
                .expect("permission request task should finish");
            assert!(matches!(
                response.outcome,
                acp::RequestPermissionOutcome::Selected(selected)
                    if selected.option_id.to_string() == "allow_once"
            ));
        })
        .await;
}

#[test]
fn default_config_uses_the_expected_delay() {
    assert_eq!(
        MockConfig::default().response_delay,
        Duration::from_millis(120)
    );
}

#[test]
fn prompt_text_formats_binary_placeholders_and_resource_links() {
    let prompt = vec![
        acp::ContentBlock::Image(acp::ImageContent::new("aGVsbG8=", "image/png")),
        acp::ContentBlock::Audio(acp::AudioContent::new("aGVsbG8=", "audio/wav")),
        acp::ContentBlock::ResourceLink(acp::ResourceLink::new("guide", "file:///guide.md")),
        acp::ContentBlock::Resource(acp::EmbeddedResource::new(
            acp::EmbeddedResourceResource::TextResourceContents(acp::TextResourceContents::new(
                "hello",
                "file:///embedded.md",
            )),
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

#[tokio::test(flavor = "current_thread")]
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

#[test]
fn finalizing_session_updates_stops_after_errors() {
    let (ack_tx, ack_rx) = oneshot::channel();

    assert!(!finalize_session_update(
        Err(acp::Error::internal_error()),
        ack_tx
    ));
    assert!(ack_rx.blocking_recv().is_err());
}

#[test]
fn finalizing_session_updates_acknowledges_successful_notifications() {
    let (ack_tx, ack_rx) = oneshot::channel();

    assert!(finalize_session_update(Ok(()), ack_tx));
    assert!(ack_rx.blocking_recv().is_ok());
}

#[tokio::test(flavor = "current_thread")]
async fn draining_session_updates_returns_after_notification_errors() {
    let call_count = Rc::new(Cell::new(0));
    let notifier = StubSessionUpdateNotifier {
        should_fail: true,
        call_count: call_count.clone(),
    };
    let (session_update_tx, session_update_rx) = mpsc::unbounded_channel();
    let (ack_tx, ack_rx) = oneshot::channel();
    session_update_tx
        .send((
            acp::SessionNotification::new(
                "mock_0",
                acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new("hello".into())),
            ),
            ack_tx,
        ))
        .expect("session update should queue");
    drop(session_update_tx);

    drain_session_updates(&notifier, session_update_rx).await;

    assert_eq!(call_count.get(), 1);
    assert!(ack_rx.await.is_err());
}
