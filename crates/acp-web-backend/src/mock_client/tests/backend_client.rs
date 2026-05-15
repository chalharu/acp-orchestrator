use super::super::backend_client::{BackendAcpClient, content_text, permission_option_ids};
use super::*;
use agent_client_protocol::{self as acp, schema};
use std::{fs, path::PathBuf, time::Duration};

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
        .create_session("alice", "w_test")
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

fn test_checkout_dir() -> PathBuf {
    let path = std::env::temp_dir().join(format!("acp-runtime-tools-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&path).expect("test checkout should be created");
    path
}

#[cfg(unix)]
fn test_chroot_checkout_dir() -> (PathBuf, PathBuf) {
    let root = test_checkout_dir();
    let chroot_root = root
        .join(crate::agent_runtime::AGENT_RUNTIMES_DIR_NAME)
        .join("session_0")
        .join("root");
    let checkout = chroot_root.join("workspace");
    fs::create_dir_all(&checkout).expect("test chroot checkout should be created");
    fs::write(chroot_root.join(".acp-test-skip-chroot-preexec"), b"")
        .expect("test chroot pre-exec marker should be created");
    (root, checkout)
}

async fn muted_client_for_checkout(checkout: PathBuf) -> BackendAcpClient {
    BackendAcpClient::new_muted_with_checkout(
        test_pending_prompt("alice", "runtime tools")
            .await
            .turn_handle(),
        checkout,
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
            if request.request_id == "req_1"
                && request.summary == "tool tool_0"
                && request.tool_call.as_ref().is_some_and(|tool| tool.tool_call_id == "tool_0")
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
async fn backend_acp_client_sanitizes_permission_tool_titles() {
    let checkout = test_checkout_dir();
    let (store, session_id, pending, mut receiver) =
        pending_permission_context("permission please").await;
    let client = BackendAcpClient::new_muted_with_checkout(pending.turn_handle(), checkout.clone());
    let title = format!("read {}", checkout.join("README.md").display());
    let requester = tokio::spawn(async move {
        client
            .request_permission(permission_request(Some(&title)))
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
            if request.summary == "read /workspace/README.md"
                && request.tool_call.as_ref().is_some_and(|tool| {
                    tool.title.as_deref() == Some("read /workspace/README.md")
                })
    ));

    store
        .resolve_permission(
            "alice",
            &session_id,
            "req_1",
            crate::contract_permissions::PermissionDecision::Deny,
        )
        .await
        .expect("permission resolution should succeed");
    let _ = requester.await.expect("permission waiter should complete");
    let _ = fs::remove_dir_all(checkout);
}

#[tokio::test(flavor = "current_thread")]
async fn backend_acp_client_streams_tool_call_events() {
    let (_store, _session_id, pending, mut receiver) =
        pending_permission_context("tool events").await;
    let checkout = test_checkout_dir();
    let client = BackendAcpClient::new_muted_with_checkout(pending.turn_handle(), checkout.clone());

    client
        .session_notification(tool_call_notification(&checkout))
        .await
        .expect("tool call should stream");
    client
        .session_notification(completed_tool_call_update_notification())
        .await
        .expect("tool update should stream");

    assert_sanitized_tool_call(
        receiver
            .recv()
            .await
            .expect("tool call event should arrive"),
    );
    assert_completed_tool_update(
        receiver
            .recv()
            .await
            .expect("tool update event should arrive"),
    );
    let _ = fs::remove_dir_all(checkout);
}

fn tool_call_notification(checkout: &std::path::Path) -> schema::SessionNotification {
    let title = format!("Read {}", checkout.join("README.md").display());
    schema::SessionNotification::new(
        "mock_0",
        schema::SessionUpdate::ToolCall(
            schema::ToolCall::new("tool_0", title)
                .kind(schema::ToolKind::Read)
                .status(schema::ToolCallStatus::InProgress)
                .raw_input(serde_json::json!({
                    checkout.join("README.md").display().to_string(): "read"
                })),
        ),
    )
}

fn completed_tool_call_update_notification() -> schema::SessionNotification {
    schema::SessionNotification::new(
        "mock_0",
        schema::SessionUpdate::ToolCallUpdate(schema::ToolCallUpdate::new(
            "tool_0",
            schema::ToolCallUpdateFields::new().status(schema::ToolCallStatus::Completed),
        )),
    )
}

fn assert_sanitized_tool_call(event: crate::contract_stream::StreamEvent) {
    assert!(matches!(
        event.payload,
        crate::contract_stream::StreamEventPayload::ToolCall { call }
            if call.tool_call_id == "tool_0"
                && call.title.as_deref() == Some("Read /workspace/README.md")
                && call.raw_input.as_ref().is_some_and(|value| {
                    value
                        .as_object()
                        .is_some_and(|object| object.contains_key("/workspace/README.md"))
                })
    ));
}

fn assert_completed_tool_update(event: crate::contract_stream::StreamEvent) {
    assert!(matches!(
        event.payload,
        crate::contract_stream::StreamEventPayload::ToolCallUpdate { update }
            if update.tool_call_id == "tool_0" && update.status.as_deref() == Some("completed")
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn backend_acp_client_read_write_text_files_are_checkout_bounded() {
    let checkout = test_checkout_dir();
    fs::write(checkout.join("README.md"), "one\ntwo\nthree\n").expect("seed file");
    fs::create_dir_all(checkout.join(".git")).expect("git dir");
    let client = muted_client_for_checkout(checkout.clone()).await;

    let read = client
        .read_text_file(
            schema::ReadTextFileRequest::new("mock_0", "/workspace/README.md")
                .line(2)
                .limit(1),
        )
        .await
        .expect("read should succeed");
    assert_eq!(read.content, "two");

    client
        .write_text_file(schema::WriteTextFileRequest::new(
            "mock_0",
            "/workspace/src/new.txt",
            "created",
        ))
        .await
        .expect("write should succeed");
    assert_eq!(
        fs::read_to_string(checkout.join("src/new.txt")).expect("written file"),
        "created"
    );

    assert_read_text_file_err(&client, "/workspace/../etc/passwd").await;
    assert_write_text_file_err(&client, "/workspace/.git/config").await;

    let _ = fs::remove_dir_all(checkout);
}

#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn backend_acp_client_rejects_filesystem_symlink_escapes() {
    let checkout = test_checkout_dir();
    let outside = test_checkout_dir();
    let client = muted_client_for_checkout(checkout.clone()).await;

    std::os::unix::fs::symlink("/etc/passwd", checkout.join("passwd_link"))
        .expect("symlink should be created");
    assert_read_text_file_err(&client, "/workspace/passwd_link").await;
    std::os::unix::fs::symlink(&outside, checkout.join("outside_link"))
        .expect("directory symlink should be created");
    assert_write_text_file_err(&client, "/workspace/outside_link/escape.txt").await;
    assert!(!outside.join("escape.txt").exists());
    std::os::unix::fs::symlink(outside.join("target.txt"), checkout.join("write_link"))
        .expect("write symlink should be created");
    assert_write_text_file_err(&client, "/workspace/write_link").await;

    let _ = fs::remove_dir_all(outside);
    let _ = fs::remove_dir_all(checkout);
}

async fn assert_read_text_file_err(client: &BackendAcpClient, path: &str) {
    assert!(
        client
            .read_text_file(schema::ReadTextFileRequest::new("mock_0", path))
            .await
            .is_err()
    );
}

async fn assert_write_text_file_err(client: &BackendAcpClient, path: &str) {
    assert!(
        client
            .write_text_file(schema::WriteTextFileRequest::new("mock_0", path, "bad"))
            .await
            .is_err()
    );
}

#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn backend_acp_client_terminal_lifecycle_is_checkout_bounded() {
    let (root, checkout) = test_chroot_checkout_dir();
    let client = muted_client_for_checkout(checkout).await;

    assert_terminal_output_is_bounded(&client).await;
    assert_terminal_cwd_escape_rejected(&client).await;
    assert_terminal_is_killable(&client).await;

    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn backend_acp_client_terminal_lifecycle_works_for_standard_checkout() {
    let checkout = test_checkout_dir();
    let client = muted_client_for_checkout(checkout.clone()).await;

    assert_terminal_output_is_bounded(&client).await;
    assert_terminal_cwd_escape_rejected(&client).await;
    assert_terminal_is_killable(&client).await;

    let _ = fs::remove_dir_all(checkout);
}

#[cfg(unix)]
async fn assert_terminal_output_is_bounded(client: &BackendAcpClient) {
    let created = client
        .create_terminal(
            schema::CreateTerminalRequest::new("mock_0", "/bin/sh")
                .args(vec!["-c".to_string(), "printf abcdef".to_string()])
                .cwd(PathBuf::from("/workspace"))
                .output_byte_limit(3),
        )
        .await
        .expect("terminal should be created");
    let terminal_id = created.terminal_id.to_string();
    let exit = client
        .wait_for_terminal_exit(schema::WaitForTerminalExitRequest::new(
            "mock_0",
            terminal_id.clone(),
        ))
        .await
        .expect("terminal should exit");
    assert_eq!(exit.exit_status.exit_code, Some(0));
    let output = client
        .terminal_output(schema::TerminalOutputRequest::new(
            "mock_0",
            terminal_id.clone(),
        ))
        .await
        .expect("terminal output should be available");
    assert_eq!(output.output, "def");
    assert!(output.truncated);
    client
        .release_terminal(schema::ReleaseTerminalRequest::new("mock_0", terminal_id))
        .await
        .expect("terminal should release");
}

#[cfg(unix)]
async fn assert_terminal_cwd_escape_rejected(client: &BackendAcpClient) {
    assert!(
        client
            .create_terminal(
                schema::CreateTerminalRequest::new("mock_0", "/bin/sh")
                    .cwd(PathBuf::from("/workspace/../"))
            )
            .await
            .is_err()
    );
}

#[cfg(unix)]
async fn assert_terminal_is_killable(client: &BackendAcpClient) {
    let sleep = client
        .create_terminal(
            schema::CreateTerminalRequest::new("mock_0", "/bin/sh")
                .args(vec!["-c".to_string(), "sleep 5".to_string()])
                .cwd(PathBuf::from("/workspace")),
        )
        .await
        .expect("sleep terminal should be created");
    client
        .kill_terminal(schema::KillTerminalRequest::new(
            "mock_0",
            sleep.terminal_id,
        ))
        .await
        .expect("terminal should be killable");
}

#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn backend_acp_client_terminal_wait_coordinates_with_kill() {
    let (root, checkout) = test_chroot_checkout_dir();
    let client = muted_client_for_checkout(checkout).await;
    let terminal_id = create_sleeping_terminal(&client).await;
    let waiter = spawn_terminal_waiter(client.clone(), terminal_id.clone());

    tokio::task::yield_now().await;
    client
        .kill_terminal(schema::KillTerminalRequest::new(
            "mock_0",
            terminal_id.clone(),
        ))
        .await
        .expect("kill should succeed while wait is pending");
    let exit = wait_for_concurrent_terminal_exit(waiter, "kill").await;
    assert_ne!(exit.exit_status.exit_code, Some(0));
    client
        .release_terminal(schema::ReleaseTerminalRequest::new("mock_0", terminal_id))
        .await
        .expect("terminal should release after concurrent kill/wait");

    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn backend_acp_client_terminal_wait_coordinates_with_release() {
    let (root, checkout) = test_chroot_checkout_dir();
    let client = muted_client_for_checkout(checkout).await;
    let terminal_id = create_sleeping_terminal(&client).await;
    let waiter = spawn_terminal_waiter(client.clone(), terminal_id.clone());

    tokio::task::yield_now().await;
    client
        .release_terminal(schema::ReleaseTerminalRequest::new("mock_0", terminal_id))
        .await
        .expect("release should succeed while wait is pending");
    let exit = wait_for_concurrent_terminal_exit(waiter, "release").await;
    assert_ne!(exit.exit_status.exit_code, Some(0));

    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
async fn create_sleeping_terminal(client: &BackendAcpClient) -> String {
    client
        .create_terminal(
            schema::CreateTerminalRequest::new("mock_0", "/bin/sh")
                .args(vec!["-c".to_string(), "sleep 5".to_string()])
                .cwd(PathBuf::from("/workspace")),
        )
        .await
        .expect("sleep terminal should be created")
        .terminal_id
        .to_string()
}

#[cfg(unix)]
fn spawn_terminal_waiter(
    client: BackendAcpClient,
    terminal_id: String,
) -> tokio::task::JoinHandle<acp::Result<schema::WaitForTerminalExitResponse>> {
    tokio::spawn(async move {
        client
            .wait_for_terminal_exit(schema::WaitForTerminalExitRequest::new(
                "mock_0",
                terminal_id,
            ))
            .await
    })
}

#[cfg(unix)]
async fn wait_for_concurrent_terminal_exit(
    waiter: tokio::task::JoinHandle<acp::Result<schema::WaitForTerminalExitResponse>>,
    action: &str,
) -> schema::WaitForTerminalExitResponse {
    tokio::time::timeout(Duration::from_secs(2), waiter)
        .await
        .unwrap_or_else(|_| panic!("wait should finish after {action}"))
        .expect("wait task should not panic")
        .expect("terminal wait should succeed")
}

#[tokio::test(flavor = "current_thread")]
async fn backend_acp_client_maps_store_errors_to_internal_errors() {
    let store = SessionStore::new(4);
    let session = store
        .create_session("alice", "w_test")
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
async fn muted_backend_acp_client_discards_chunks_until_streaming_starts() {
    let (_store, _session_id, pending, mut receiver) =
        pending_permission_context("stream please").await;
    let client = BackendAcpClient::new_muted(pending.turn_handle());

    client
        .session_notification(schema::SessionNotification::new(
            "mock_0",
            schema::SessionUpdate::AgentMessageChunk(schema::ContentChunk::new("replay".into())),
        ))
        .await
        .expect("muted session updates should be accepted");
    client.enable_streaming();
    client
        .session_notification(schema::SessionNotification::new(
            "mock_0",
            schema::SessionUpdate::AgentMessageChunk(schema::ContentChunk::new("fresh".into())),
        ))
        .await
        .expect("streaming session updates should be accepted");

    assert_eq!(client.reply_text(), "fresh");
    let event = receiver
        .recv()
        .await
        .expect("fresh assistant chunk should be published");
    assert!(matches!(
        event.payload,
        crate::contract_stream::StreamEventPayload::ConversationMessage { message, .. }
            if message.text == "fresh"
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn prompt_response_notification_wait_stops_after_grace_deadline() {
    let client = BackendAcpClient::without_turn();
    let expired_deadline = tokio::time::Instant::now()
        .checked_sub(Duration::from_millis(1))
        .expect("past instants should be representable");

    client
        .wait_for_response_notifications_until_for_test(expired_deadline)
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn backend_acp_client_streams_agent_message_chunks_to_session_events() {
    let (_store, _session_id, pending, mut receiver) =
        pending_permission_context("stream please").await;
    let client = BackendAcpClient::new(pending.turn_handle());

    client
        .session_notification(schema::SessionNotification::new(
            "mock_0",
            schema::SessionUpdate::AgentMessageChunk(schema::ContentChunk::new("chunk".into())),
        ))
        .await
        .expect("session updates should stream");

    let event = receiver
        .recv()
        .await
        .expect("assistant chunk should be published");
    assert!(matches!(
        event.payload,
        crate::contract_stream::StreamEventPayload::ConversationMessage { message, .. }
            if message.text == "chunk"
    ));
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
