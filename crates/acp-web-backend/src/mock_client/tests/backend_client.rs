use super::super::backend_client::{BackendAcpClient, content_text, permission_option_ids};
use super::*;

fn permission_options() -> Vec<acp::PermissionOption> {
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
    ]
}

async fn pending_permission_context(
    summary: &str,
) -> (
    SessionStore,
    String,
    PendingPrompt,
    tokio::sync::broadcast::Receiver<acp_contracts::StreamEvent>,
) {
    let store = SessionStore::new(4);
    let session = store
        .create_session("alice")
        .await
        .expect("session creation should succeed");
    let session_id = session.id.clone();
    let (_snapshot, mut receiver) = store
        .session_events("alice", &session_id)
        .await
        .expect("subscribing should succeed");
    let pending = store
        .submit_prompt("alice", &session_id, summary.to_string())
        .await
        .expect("prompt submission should succeed");
    let _ = receiver.recv().await.expect("user event should arrive");
    let _cancel_rx = pending
        .turn_handle()
        .start_turn()
        .await
        .expect("starting the turn should succeed");
    (store, session_id, pending, receiver)
}

fn permission_request(title: Option<&str>) -> acp::RequestPermissionRequest {
    let mut fields = acp::ToolCallUpdateFields::new();
    if let Some(title) = title {
        fields = fields.title(title);
    }
    acp::RequestPermissionRequest::new(
        "mock_0",
        acp::ToolCallUpdate::new("tool_0", fields),
        permission_options(),
    )
}

#[tokio::test(flavor = "current_thread")]
async fn backend_acp_client_rejects_invalid_permission_requests() {
    let client = BackendAcpClient::new(
        test_pending_prompt("alice", "permission please")
            .await
            .turn_handle(),
    );
    let error = client
        .request_permission(acp::RequestPermissionRequest::new(
            "mock_0",
            acp::ToolCallUpdate::new(
                "tool_0",
                acp::ToolCallUpdateFields::new().title("permission prompt"),
            ),
            vec![acp::PermissionOption::new(
                "allow_once",
                "Allow once",
                acp::PermissionOptionKind::AllowOnce,
            )],
        ))
        .await
        .expect_err("missing deny options should be rejected");

    assert_eq!(error.message, acp::Error::invalid_params().message);
}

#[tokio::test(flavor = "current_thread")]
async fn backend_acp_client_uses_the_tool_call_id_when_titles_are_missing() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let (store, session_id, pending, mut receiver) =
                pending_permission_context("permission please").await;
            let client = BackendAcpClient::new(pending.turn_handle());
            let requester = tokio::task::spawn_local(async move {
                client
                    .request_permission(permission_request(None))
                    .await
                    .expect("permission requests should resolve")
            });

            let permission_event = receiver
                .recv()
                .await
                .expect("permission event should arrive");
            assert!(matches!(
                permission_event.payload,
                acp_contracts::StreamEventPayload::PermissionRequested { request }
                    if request.request_id == "req_1" && request.summary == "tool tool_0"
            ));

            let resolved = store
                .resolve_permission(
                    "alice",
                    &session_id,
                    "req_1",
                    acp_contracts::PermissionDecision::Deny,
                )
                .await
                .expect("permission resolution should succeed");
            assert_eq!(resolved.request_id, "req_1");

            let response = requester.await.expect("permission waiter should complete");
            assert!(matches!(
                response.outcome,
                acp::RequestPermissionOutcome::Selected(selected)
                    if selected.option_id.to_string() == "reject_once"
            ));
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn backend_acp_client_maps_store_errors_to_internal_errors() {
    let store = SessionStore::new(4);
    let session = store
        .create_session("alice")
        .await
        .expect("session creation should succeed");
    let pending = store
        .submit_prompt("alice", &session.id, "permission please".to_string())
        .await
        .expect("prompt submission should succeed");
    store
        .close_session("alice", &session.id)
        .await
        .expect("closing the session should succeed");
    let client = BackendAcpClient::new(pending.turn_handle());

    let error = client
        .request_permission(permission_request(Some("permission prompt")))
        .await
        .expect_err("closed sessions should map to ACP internal errors");

    assert_eq!(error.message, acp::Error::internal_error().message);
}

#[tokio::test(flavor = "current_thread")]
async fn backend_acp_client_collects_agent_message_chunks() {
    let client = BackendAcpClient::new(test_pending_prompt("alice", "hello").await.turn_handle());

    client
        .session_notification(acp::SessionNotification::new(
            "mock_0",
            acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new("first chunk".into())),
        ))
        .await
        .expect("session updates should succeed");

    assert_eq!(client.reply_text(), "first chunk");
}

#[tokio::test(flavor = "current_thread")]
async fn backend_acp_client_waits_for_permission_decisions() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let (store, session_id, pending, _receiver) =
                pending_permission_context("permission please").await;
            let client = BackendAcpClient::new(pending.turn_handle());
            let requester = tokio::task::spawn_local(async move {
                client
                    .request_permission(permission_request(Some("read_text_file README.md")))
                    .await
                    .expect("permission request should resolve")
            });

            tokio::task::yield_now().await;
            let resolution = store
                .resolve_permission(
                    "alice",
                    &session_id,
                    "req_1",
                    acp_contracts::PermissionDecision::Approve,
                )
                .await
                .expect("permission resolution should succeed");
            assert_eq!(resolution.request_id, "req_1");

            let response = requester.await.expect("permission waiter should complete");
            assert!(matches!(
                response.outcome,
                acp::RequestPermissionOutcome::Selected(selected)
                    if selected.option_id.to_string() == "allow_once"
            ));
        })
        .await;
}

#[test]
fn permission_option_ids_require_allow_and_deny_choices() {
    let request = acp::RequestPermissionRequest::new(
        "mock_0",
        acp::ToolCallUpdate::new(
            "tool_0",
            acp::ToolCallUpdateFields::new().title("permission prompt"),
        ),
        vec![acp::PermissionOption::new(
            "allow_once",
            "Allow once",
            acp::PermissionOptionKind::AllowOnce,
        )],
    );

    assert!(matches!(
        permission_option_ids(&request),
        Err(MockClientError::InvalidPermissionOptions)
    ));
}

#[test]
fn permission_option_ids_reject_persistent_permission_choices() {
    let request = acp::RequestPermissionRequest::new(
        "mock_0",
        acp::ToolCallUpdate::new(
            "tool_0",
            acp::ToolCallUpdateFields::new().title("permission prompt"),
        ),
        vec![
            acp::PermissionOption::new(
                "allow_always",
                "Allow always",
                acp::PermissionOptionKind::AllowAlways,
            ),
            acp::PermissionOption::new(
                "reject_once",
                "Reject once",
                acp::PermissionOptionKind::RejectOnce,
            ),
        ],
    );

    assert!(matches!(
        permission_option_ids(&request),
        Err(MockClientError::UnsupportedPermissionOptions)
    ));
}

#[tokio::test]
async fn default_reply_provider_cleanup_is_a_no_op() {
    #[derive(Debug)]
    struct NoopProvider;

    impl ReplyProvider for NoopProvider {
        fn request_reply<'a>(&'a self, _turn: TurnHandle) -> ReplyFuture<'a> {
            Box::pin(async { Ok(ReplyResult::NoOutput) })
        }
    }

    let reply = NoopProvider
        .request_reply(test_pending_prompt("alice", "hello").await.turn_handle())
        .await
        .expect("the default no-op provider should return without error");
    assert_eq!(reply, ReplyResult::NoOutput);
    NoopProvider.forget_session("s_test");
}

#[test]
fn permission_option_ids_reject_duplicate_once_choices() {
    let request = acp::RequestPermissionRequest::new(
        "mock_0",
        acp::ToolCallUpdate::new(
            "tool_0",
            acp::ToolCallUpdateFields::new().title("permission prompt"),
        ),
        vec![
            acp::PermissionOption::new(
                "allow_once_1",
                "Allow once",
                acp::PermissionOptionKind::AllowOnce,
            ),
            acp::PermissionOption::new(
                "allow_once_2",
                "Allow once again",
                acp::PermissionOptionKind::AllowOnce,
            ),
            acp::PermissionOption::new(
                "reject_once",
                "Reject once",
                acp::PermissionOptionKind::RejectOnce,
            ),
        ],
    );

    assert!(matches!(
        permission_option_ids(&request),
        Err(MockClientError::UnsupportedPermissionOptions)
    ));
}

#[test]
fn content_text_formats_embedded_resources() {
    let resource = acp::ContentBlock::Resource(acp::EmbeddedResource::new(
        acp::EmbeddedResourceResource::TextResourceContents(acp::TextResourceContents::new(
            "hello",
            "file:///embedded.md",
        )),
    ));

    assert_eq!(content_text(resource), "<resource>");
}

#[test]
fn content_text_formats_non_text_prompt_blocks() {
    assert_eq!(
        content_text(acp::ContentBlock::Image(acp::ImageContent::new(
            "aGVsbG8=",
            "image/png",
        ))),
        "<image>"
    );
    assert_eq!(
        content_text(acp::ContentBlock::Audio(acp::AudioContent::new(
            "aGVsbG8=",
            "audio/wav",
        ))),
        "<audio>"
    );
    assert_eq!(
        content_text(acp::ContentBlock::ResourceLink(acp::ResourceLink::new(
            "guide",
            "file:///guide.md",
        ))),
        "file:///guide.md"
    );
}
