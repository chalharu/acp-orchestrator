use super::super::backend_client::{BackendAcpClient, content_text, permission_option_ids};
use super::*;
use agent_client_protocol::schema;

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

async fn pending_permission_context(
    summary: &str,
) -> (
    SessionStore,
    String,
    PendingPrompt,
    tokio::sync::broadcast::Receiver<crate::contract_stream::StreamEvent>,
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

fn permission_request(title: Option<&str>) -> schema::RequestPermissionRequest {
    let mut fields = schema::ToolCallUpdateFields::new();
    if let Some(title) = title {
        fields = fields.title(title);
    }
    schema::RequestPermissionRequest::new(
        "mock_0",
        schema::ToolCallUpdate::new("tool_0", fields),
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
        .request_permission(schema::RequestPermissionRequest::new(
            "mock_0",
            schema::ToolCallUpdate::new(
                "tool_0",
                schema::ToolCallUpdateFields::new().title("permission prompt"),
            ),
            vec![schema::PermissionOption::new(
                "allow_once",
                "Allow once",
                schema::PermissionOptionKind::AllowOnce,
            )],
        ))
        .await
        .expect_err("missing deny options should be rejected");

    assert_eq!(error.message, acp::Error::invalid_params().message);
}

#[tokio::test(flavor = "current_thread")]
async fn backend_acp_client_uses_the_tool_call_id_when_titles_are_missing() {
    let (store, session_id, pending, mut receiver) =
        pending_permission_context("permission please").await;
    let client = BackendAcpClient::new(pending.turn_handle());
    let requester = tokio::spawn(async move {
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
        crate::contract_stream::StreamEventPayload::PermissionRequested { request }
            if request.request_id == "req_1" && request.summary == "tool tool_0"
    ));

    let resolved = store
        .resolve_permission(
            "alice",
            &session_id,
            "req_1",
            crate::contract_permissions::PermissionDecision::Deny,
        )
        .await
        .expect("permission resolution should succeed");
    assert_eq!(resolved.request_id, "req_1");

    let response = requester.await.expect("permission waiter should complete");
    assert!(matches!(
        response.outcome,
        schema::RequestPermissionOutcome::Selected(selected)
            if selected.option_id.to_string() == "reject_once"
    ));
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
        .session_notification(schema::SessionNotification::new(
            "mock_0",
            schema::SessionUpdate::AgentMessageChunk(schema::ContentChunk::new(
                "first chunk".into(),
            )),
        ))
        .await
        .expect("session updates should succeed");

    assert_eq!(client.reply_text(), "first chunk");
}

#[tokio::test(flavor = "current_thread")]
async fn backend_acp_client_waits_for_permission_decisions() {
    let (store, session_id, pending, _receiver) =
        pending_permission_context("permission please").await;
    let client = BackendAcpClient::new(pending.turn_handle());
    let requester = tokio::spawn(async move {
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
            crate::contract_permissions::PermissionDecision::Approve,
        )
        .await
        .expect("permission resolution should succeed");
    assert_eq!(resolution.request_id, "req_1");

    let response = requester.await.expect("permission waiter should complete");
    assert!(matches!(
        response.outcome,
        schema::RequestPermissionOutcome::Selected(selected)
            if selected.option_id.to_string() == "allow_once"
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn backend_acp_client_returns_cancelled_outcomes_when_turns_are_cancelled() {
    let (store, session_id, pending, _receiver) =
        pending_permission_context("permission please").await;
    let client = BackendAcpClient::new(pending.turn_handle());
    let requester = tokio::spawn(async move {
        client
            .request_permission(permission_request(Some("read_text_file README.md")))
            .await
            .expect("permission request should resolve")
    });

    tokio::task::yield_now().await;
    assert!(
        store
            .cancel_active_turn("alice", &session_id)
            .await
            .expect("cancelling should succeed"),
        "the active turn should have started"
    );

    let response = requester.await.expect("permission waiter should complete");
    assert!(matches!(
        response.outcome,
        schema::RequestPermissionOutcome::Cancelled
    ));
}

#[test]
fn permission_option_ids_require_allow_and_deny_choices() {
    let request = schema::RequestPermissionRequest::new(
        "mock_0",
        schema::ToolCallUpdate::new(
            "tool_0",
            schema::ToolCallUpdateFields::new().title("permission prompt"),
        ),
        vec![schema::PermissionOption::new(
            "allow_once",
            "Allow once",
            schema::PermissionOptionKind::AllowOnce,
        )],
    );

    assert!(matches!(
        permission_option_ids(&request),
        Err(MockClientError::InvalidPermissionOptions)
    ));
}

#[test]
fn permission_option_ids_reject_persistent_permission_choices() {
    let request = schema::RequestPermissionRequest::new(
        "mock_0",
        schema::ToolCallUpdate::new(
            "tool_0",
            schema::ToolCallUpdateFields::new().title("permission prompt"),
        ),
        vec![
            schema::PermissionOption::new(
                "allow_always",
                "Allow always",
                schema::PermissionOptionKind::AllowAlways,
            ),
            schema::PermissionOption::new(
                "reject_once",
                "Reject once",
                schema::PermissionOptionKind::RejectOnce,
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
fn default_reply_provider_prime_session_is_a_no_op() {
    #[derive(Debug)]
    struct NoopProvider;

    impl ReplyProvider for NoopProvider {
        fn request_reply<'a>(&'a self, _turn: TurnHandle) -> ReplyFuture<'a> {
            Box::pin(async { Ok(ReplyResult::NoOutput) })
        }
    }

    let runtime = tokio::runtime::Runtime::new().expect("runtime should build");
    let hint = runtime
        .block_on(NoopProvider.prime_session("s_test"))
        .expect("the default prime-session hook should succeed");

    assert_eq!(hint, None);
}

#[test]
fn permission_option_ids_reject_duplicate_once_choices() {
    let request = schema::RequestPermissionRequest::new(
        "mock_0",
        schema::ToolCallUpdate::new(
            "tool_0",
            schema::ToolCallUpdateFields::new().title("permission prompt"),
        ),
        vec![
            schema::PermissionOption::new(
                "allow_once_1",
                "Allow once",
                schema::PermissionOptionKind::AllowOnce,
            ),
            schema::PermissionOption::new(
                "allow_once_2",
                "Allow once again",
                schema::PermissionOptionKind::AllowOnce,
            ),
            schema::PermissionOption::new(
                "reject_once",
                "Reject once",
                schema::PermissionOptionKind::RejectOnce,
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
    let resource = schema::ContentBlock::Resource(schema::EmbeddedResource::new(
        schema::EmbeddedResourceResource::TextResourceContents(schema::TextResourceContents::new(
            "hello",
            "file:///embedded.md",
        )),
    ));

    assert_eq!(content_text(resource), "<resource>");
}

#[test]
fn content_text_formats_non_text_prompt_blocks() {
    assert_eq!(
        content_text(schema::ContentBlock::Image(schema::ImageContent::new(
            "aGVsbG8=",
            "image/png",
        ))),
        "<image>"
    );
    assert_eq!(
        content_text(schema::ContentBlock::Audio(schema::AudioContent::new(
            "aGVsbG8=",
            "audio/wav",
        ))),
        "<audio>"
    );
    assert_eq!(
        content_text(schema::ContentBlock::ResourceLink(
            schema::ResourceLink::new("guide", "file:///guide.md",)
        )),
        "file:///guide.md"
    );
}
