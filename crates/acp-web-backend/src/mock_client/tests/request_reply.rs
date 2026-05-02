use super::*;
use crate::agent_runtime::{AgentLaunchMetadata, AgentStdioMetadata};
use crate::contract_permissions::PermissionDecision;
use crate::contract_stream::StreamEventPayload;
use crate::sessions::TurnHandle;
use std::path::{Path, PathBuf};

const FAKE_STDIO_AGENT_SCRIPT: &str = r#"
import json
import sys

session_id = "stdio_0"

def send(message):
    print(json.dumps(message), flush=True)

for line in sys.stdin:
    request = json.loads(line)
    method = request.get("method")
    request_id = request.get("id")
    if request_id is None:
        continue
    if method == "initialize":
        send({
            "jsonrpc": "2.0",
            "id": request_id,
            "result": {
                "protocolVersion": 1,
                "agentCapabilities": {"loadSession": False},
                "agentInfo": {
                    "name": "stdio-test",
                    "title": "Stdio Test",
                    "version": "0"
                }
            }
        })
    elif method == "session/new":
        send({"jsonrpc": "2.0", "id": request_id, "result": {"sessionId": session_id}})
    elif method == "session/prompt":
        send({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": session_id,
                "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": "stdio reply"}
                }
            }
        })
        send({"jsonrpc": "2.0", "id": request_id, "result": {"stopReason": "end_turn"}})
    else:
        send({
            "jsonrpc": "2.0",
            "id": request_id,
            "error": {"code": -32601, "message": "method not found"}
        })
"#;

const FAKE_STDIO_LOAD_REPLAY_SCRIPT: &str = r#"
import json
import sys

session_id = "stdio_0"

def send(message):
    print(json.dumps(message), flush=True)

def send_chunk(text):
    send({
        "jsonrpc": "2.0",
        "method": "session/update",
        "params": {
            "sessionId": session_id,
            "update": {
                "sessionUpdate": "agent_message_chunk",
                "content": {"type": "text", "text": text}
            }
        }
    })

for line in sys.stdin:
    request = json.loads(line)
    method = request.get("method")
    request_id = request.get("id")
    if request_id is None:
        continue
    if method == "initialize":
        send({
            "jsonrpc": "2.0",
            "id": request_id,
            "result": {
                "protocolVersion": 1,
                "agentCapabilities": {"loadSession": True},
                "agentInfo": {"name": "stdio-test", "title": "Stdio Test", "version": "0"}
            }
        })
    elif method == "session/new":
        send({"jsonrpc": "2.0", "id": request_id, "result": {"sessionId": session_id}})
    elif method == "session/load":
        send_chunk("replayed old reply")
        send({"jsonrpc": "2.0", "id": request_id, "result": {}})
    elif method == "session/prompt":
        send_chunk("fresh reply")
        send({"jsonrpc": "2.0", "id": request_id, "result": {"stopReason": "end_turn"}})
    else:
        send({"jsonrpc": "2.0", "id": request_id, "error": {"code": -32601, "message": "method not found"}})
"#;

async fn permission_roundtrip_context() -> (
    MockClient,
    SessionStore,
    String,
    tokio::sync::broadcast::Receiver<crate::contract_stream::StreamEvent>,
    crate::sessions::PendingPrompt,
    tokio::sync::oneshot::Sender<()>,
) {
    let (mock_address, shutdown_tx) = spawn_mock_server(Duration::from_millis(1)).await;
    let client = MockClient::new(mock_address).expect("client construction should succeed");
    let store = SessionStore::new(4);
    let session = store
        .create_session("alice", "w_test")
        .await
        .expect("session creation should succeed");
    let (_snapshot, receiver) = store
        .session_events("alice", &session.id)
        .await
        .expect("session subscriptions should succeed");
    let pending = store
        .submit_prompt(
            "alice",
            &session.id,
            acp_mock::MANUAL_PERMISSION_TRIGGER.to_string(),
        )
        .await
        .expect("prompt submission should succeed");

    (client, store, session.id, receiver, pending, shutdown_tx)
}

async fn expect_permission_requested(
    receiver: &mut tokio::sync::broadcast::Receiver<crate::contract_stream::StreamEvent>,
) {
    let permission_event = receiver
        .recv()
        .await
        .expect("permission requests should be published");
    assert!(matches!(
        permission_event.payload,
        StreamEventPayload::PermissionRequested { request }
            if request.request_id == "req_1"
                && request.summary == "read_text_file README.md"
    ));
}

async fn approve_permission_request(
    store: &SessionStore,
    session_id: &str,
    receiver: &mut tokio::sync::broadcast::Receiver<crate::contract_stream::StreamEvent>,
) {
    store
        .resolve_permission("alice", session_id, "req_1", PermissionDecision::Approve)
        .await
        .expect("permission approvals should succeed");
    let _ = receiver
        .recv()
        .await
        .expect("permission snapshots should be published");
}

fn temp_stdio_working_dir() -> PathBuf {
    std::env::temp_dir().join(format!("acp-stdio-agent-{}", uuid::Uuid::new_v4().simple()))
}

fn python3_path() -> String {
    ["/usr/bin/python3", "/usr/local/bin/python3"]
        .into_iter()
        .find(|path| std::path::Path::new(path).exists())
        .unwrap_or("python3")
        .to_string()
}

async fn bind_stdio_script(
    client: &MockClient,
    session_id: &str,
    working_dir: &Path,
    script: &str,
) {
    let working_dir = working_dir.to_path_buf();
    client
        .bind_session(session_id, working_dir.clone())
        .await
        .expect("working dir bind should succeed");
    client
        .bind_session_launch_metadata(
            session_id,
            AgentLaunchMetadata {
                acp_address: None,
                stdio: Some(AgentStdioMetadata {
                    argv: vec![
                        python3_path(),
                        "-u".to_string(),
                        "-c".to_string(),
                        script.to_string(),
                    ],
                    env: Vec::new(),
                    working_dir: working_dir.clone(),
                }),
            },
        )
        .await
        .expect("stdio metadata bind should succeed");
}

#[tokio::test]
async fn request_reply_collects_text_from_acp_mock() {
    let (mock_address, shutdown_tx) = spawn_mock_server(Duration::from_millis(1)).await;
    let client = MockClient::new(mock_address).expect("client construction should succeed");
    let pending = test_pending_prompt("alice", "hello").await;

    let reply = client
        .request_reply(pending.turn_handle())
        .await
        .expect("mock ACP replies should succeed");

    assert!(matches!(
        reply,
        ReplyResult::Reply(text) if text.starts_with("mock assistant:")
    ));

    let _ = shutdown_tx.send(());
}

#[tokio::test]
async fn request_reply_can_drive_stdio_acp_agents() {
    let client = MockClient::with_timeout("127.0.0.1:9".to_string(), Duration::from_secs(3))
        .expect("client construction should succeed");
    let store = SessionStore::new(4);
    let session = store
        .create_session("alice", "w_test")
        .await
        .expect("session creation should succeed");
    let working_dir = temp_stdio_working_dir();
    std::fs::create_dir_all(&working_dir).expect("stdio working dir should be created");
    bind_stdio_script(&client, &session.id, &working_dir, FAKE_STDIO_AGENT_SCRIPT).await;
    let pending = store
        .submit_prompt("alice", &session.id, "hello".to_string())
        .await
        .expect("prompt submission should succeed");

    let reply = client
        .request_reply(pending.turn_handle())
        .await
        .expect("stdio ACP replies should succeed");

    assert_eq!(reply, ReplyResult::Reply("stdio reply".to_string()));
    let _ = std::fs::remove_dir_all(working_dir);
}

#[tokio::test]
async fn request_reply_ignores_session_load_replay_chunks() {
    let client = MockClient::with_timeout("127.0.0.1:9".to_string(), Duration::from_secs(3))
        .expect("client construction should succeed");
    let store = SessionStore::new(4);
    let session = store
        .create_session("alice", "w_test")
        .await
        .expect("session creation should succeed");
    let working_dir = temp_stdio_working_dir();
    std::fs::create_dir_all(&working_dir).expect("stdio working dir should be created");
    bind_stdio_script(
        &client,
        &session.id,
        &working_dir,
        FAKE_STDIO_LOAD_REPLAY_SCRIPT,
    )
    .await;

    request_stdio_reply(&client, &store, &session.id, "first")
        .await
        .expect("first stdio prompt should succeed");
    let reply = request_stdio_reply(&client, &store, &session.id, "second")
        .await
        .expect("loaded stdio prompt should succeed");

    assert_eq!(reply, ReplyResult::Reply("fresh reply".to_string()));
    let _ = std::fs::remove_dir_all(working_dir);
}

async fn request_stdio_reply(
    client: &MockClient,
    store: &SessionStore,
    session_id: &str,
    prompt: &str,
) -> Result<ReplyResult> {
    let pending = store
        .submit_prompt("alice", session_id, prompt.to_string())
        .await
        .expect("prompt submission should succeed");
    client.request_reply(pending.turn_handle()).await
}

#[tokio::test]
async fn prime_session_hint_collects_startup_hints_without_polluting_prompt_replies() {
    let (mock_address, shutdown_tx) = spawn_mock_server_with_config(MockConfig {
        response_delay: Duration::from_millis(1),
        startup_hints: true,
        ..MockConfig::default()
    })
    .await;
    let client = MockClient::new(mock_address).expect("client construction should succeed");
    let store = SessionStore::new(4);
    let session = store
        .create_session("alice", "w_test")
        .await
        .expect("session creation should succeed");

    let hint = client
        .prime_session_hint(&session.id)
        .await
        .expect("session priming should succeed")
        .expect("startup hints should be returned");
    assert!(hint.contains("verify permission"));
    assert_eq!(
        client.mapped_session_id(&session.id).await.as_deref(),
        Some("mock_0")
    );

    let pending = store
        .submit_prompt("alice", &session.id, "hello".to_string())
        .await
        .expect("prompt submission should succeed");
    let reply = client
        .request_reply(pending.turn_handle())
        .await
        .expect("mock ACP replies should succeed");
    assert!(matches!(
        reply,
        ReplyResult::Reply(text) if text.starts_with("mock assistant:")
    ));

    let _ = shutdown_tx.send(());
}

#[tokio::test]
async fn prime_session_hint_authenticates_before_creating_sessions() {
    let (mock_address, shutdown_tx) = spawn_mock_server_with_config(MockConfig {
        response_delay: Duration::from_millis(1),
        auth_required: true,
        ..MockConfig::default()
    })
    .await;
    let client = MockClient::new(mock_address).expect("client construction should succeed");
    let pending = test_pending_prompt("alice", "hello").await;
    let session_id = pending.turn_handle().session_id().to_string();

    let hint = client
        .prime_session_hint(&session_id)
        .await
        .expect("authenticated session priming should succeed");

    assert!(hint.is_none());
    assert_eq!(
        client.mapped_session_id(&session_id).await.as_deref(),
        Some("mock_0")
    );
    let _ = shutdown_tx.send(());
}

#[tokio::test]
async fn request_reply_reuses_upstream_sessions_for_the_same_backend_session() {
    let (mock_address, shutdown_tx) = spawn_mock_server(Duration::from_millis(1)).await;
    let client = MockClient::new(mock_address).expect("client construction should succeed");
    let store = SessionStore::new(4);
    let session = store
        .create_session("alice", "w_test")
        .await
        .expect("session creation should succeed");
    let first = store
        .submit_prompt("alice", &session.id, "first prompt".to_string())
        .await
        .expect("first prompt should submit");
    let second = store
        .submit_prompt("alice", &session.id, "second prompt".to_string())
        .await
        .expect("second prompt should submit");
    let other_session = store
        .create_session("bob", "w_test")
        .await
        .expect("second session should succeed");
    let other_session_id = other_session.id.clone();
    let third = store
        .submit_prompt("bob", &other_session_id, "third prompt".to_string())
        .await
        .expect("third prompt should submit");

    client
        .request_reply(first.turn_handle())
        .await
        .expect("first replies should succeed");
    client
        .request_reply(second.turn_handle())
        .await
        .expect("reused sessions should succeed");
    client
        .request_reply(third.turn_handle())
        .await
        .expect("second backend sessions should succeed");

    assert_eq!(
        client.mapped_session_id(&session.id).await.as_deref(),
        Some("mock_0")
    );
    assert_eq!(
        client.mapped_session_id(&other_session_id).await.as_deref(),
        Some("mock_1")
    );

    let _ = shutdown_tx.send(());
}

#[tokio::test]
async fn forgetting_sessions_clears_cached_upstream_state() {
    let (mock_address, shutdown_tx) = spawn_mock_server(Duration::from_millis(1)).await;
    let client = MockClient::new(mock_address).expect("client construction should succeed");
    let store = SessionStore::new(4);
    let session = store
        .create_session("alice", "w_test")
        .await
        .expect("session creation should succeed");
    let pending = store
        .submit_prompt("alice", &session.id, "hello".to_string())
        .await
        .expect("prompt submission should succeed");

    client
        .request_reply(pending.turn_handle())
        .await
        .expect("mock ACP replies should succeed");
    assert_eq!(
        client.mapped_session_id(&session.id).await.as_deref(),
        Some("mock_0")
    );

    MockClient::forget_session(&client, &session.id).await;

    assert_eq!(client.mapped_session_id(&session.id).await, None);
    assert!(
        client.session_locks.lock().await.get(&session.id).is_none(),
        "session locks should be released with the upstream cache entry"
    );

    let _ = shutdown_tx.send(());
}

#[tokio::test]
async fn request_reply_times_out_for_slow_mock_agents() {
    let (mock_address, shutdown_tx) = spawn_mock_server(Duration::from_millis(200)).await;
    let client = MockClient::with_timeout(mock_address, Duration::from_millis(20))
        .expect("client construction should succeed");
    let pending = test_pending_prompt("alice", "hello").await;

    let error = client
        .request_reply(pending.turn_handle())
        .await
        .expect_err("stalled responses should time out");

    assert!(matches!(error, MockClientError::TimedOut { .. }));

    let _ = shutdown_tx.send(());
}

#[tokio::test]
async fn request_reply_reports_connect_failures() {
    let client = MockClient::with_timeout("127.0.0.1:9".to_string(), Duration::from_millis(200))
        .expect("client construction should succeed");
    let pending = test_pending_prompt("alice", "hello").await;

    let error = client
        .request_reply(pending.turn_handle())
        .await
        .expect_err("unreachable mock transports should fail");

    assert!(matches!(error, MockClientError::Connect { .. }));
}

#[tokio::test]
async fn request_reply_retries_until_launched_acp_server_is_listening() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("port reservation should bind");
    let address = listener
        .local_addr()
        .expect("listener should expose address");
    drop(listener);
    let client = MockClient::with_timeout(address.to_string(), Duration::from_secs(2))
        .expect("client construction should succeed");
    let pending = test_pending_prompt("alice", "hello").await;
    let request_task =
        tokio::spawn(async move { client.request_reply(pending.turn_handle()).await });

    tokio::time::sleep(Duration::from_millis(50)).await;
    let listener = TcpListener::bind(address)
        .await
        .expect("delayed mock server should bind the reserved address");
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    spawn_with_shutdown_task(
        listener,
        MockConfig {
            response_delay: Duration::from_millis(1),
            ..MockConfig::default()
        },
        async move {
            let _ = shutdown_rx.await;
        },
    );

    let reply = request_task
        .await
        .expect("request task should join")
        .expect("late ACP listeners should still receive the request");
    assert!(matches!(
        reply,
        ReplyResult::Reply(text) if text.starts_with("mock assistant:")
    ));
    let _ = shutdown_tx.send(());
}

#[tokio::test]
async fn request_reply_returns_cancelled_status_when_turns_are_cancelled() {
    let (mock_address, shutdown_tx) = spawn_mock_server(Duration::from_millis(200)).await;
    let client = MockClient::new(mock_address).expect("client construction should succeed");
    let store = SessionStore::new(4);
    let session = store
        .create_session("alice", "w_test")
        .await
        .expect("session creation should succeed");
    let pending = store
        .submit_prompt("alice", &session.id, "hello".to_string())
        .await
        .expect("prompt submission should succeed");

    let turn = pending.turn_handle();
    let request_task = {
        let client = client.clone();
        let turn = turn.clone();
        tokio::spawn(async move { client.request_reply(turn).await })
    };

    wait_for_turn_to_start(&turn).await;
    assert!(
        store
            .cancel_active_turn("alice", &session.id)
            .await
            .expect("cancelling should succeed"),
        "the active turn should have started"
    );

    let reply = request_task
        .await
        .expect("request task should join")
        .expect("cancelled turns should resolve cleanly");
    assert_eq!(reply, ReplyResult::Status("turn cancelled".to_string()));

    let _ = shutdown_tx.send(());
}

#[tokio::test]
async fn request_reply_roundtrips_permission_requests_through_acp() {
    let (client, store, session_id, mut receiver, pending, shutdown_tx) =
        permission_roundtrip_context().await;

    let _ = receiver
        .recv()
        .await
        .expect("user prompt events should arrive");
    let request_task = {
        let client = client.clone();
        let turn = pending.turn_handle();
        tokio::spawn(async move { client.request_reply(turn).await })
    };

    expect_permission_requested(&mut receiver).await;
    approve_permission_request(&store, &session_id, &mut receiver).await;

    let reply = request_task
        .await
        .expect("request task should join")
        .expect("permission-gated replies should succeed");
    assert!(matches!(
        reply,
        ReplyResult::Reply(text) if text.starts_with("mock assistant:")
    ));

    let _ = shutdown_tx.send(());
}

async fn wait_for_turn_to_start(turn: &TurnHandle) {
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if turn.is_started().await {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the active turn should have started within the timeout");
}

#[tokio::test]
async fn request_reply_continues_when_the_cancel_watch_is_replaced_without_cancelling() {
    let (mock_address, shutdown_tx) = spawn_mock_server(Duration::from_millis(100)).await;
    let client = MockClient::new(mock_address).expect("client construction should succeed");
    let store = SessionStore::new(4);
    let session = store
        .create_session("alice", "w_test")
        .await
        .expect("session creation should succeed");
    let first = store
        .submit_prompt("alice", &session.id, "first".to_string())
        .await
        .expect("first prompt submission should succeed");
    let second = store
        .submit_prompt("alice", &session.id, "second".to_string())
        .await
        .expect("second prompt submission should succeed");

    let request_task = {
        let client = client.clone();
        tokio::spawn(async move { client.request_reply(first.turn_handle()).await })
    };

    tokio::time::sleep(Duration::from_millis(20)).await;
    let _ = second
        .turn_handle()
        .start_turn()
        .await
        .expect("starting a later turn should replace the active watch");

    let reply = request_task
        .await
        .expect("request task should join")
        .expect("the prompt should still resolve after the watch is replaced");
    assert!(matches!(
        reply,
        ReplyResult::Reply(text) if text.starts_with("mock assistant:")
    ));

    let _ = shutdown_tx.send(());
}

#[tokio::test]
async fn request_reply_reports_session_runtime_failures_after_sessions_close() {
    let (mock_address, shutdown_tx) = spawn_mock_server(Duration::from_millis(1)).await;
    let client = MockClient::new(mock_address).expect("client construction should succeed");
    let store = SessionStore::new(4);
    let session = store
        .create_session("alice", "w_test")
        .await
        .expect("session creation should succeed");
    let pending = store
        .submit_prompt("alice", &session.id, "hello".to_string())
        .await
        .expect("prompt submission should succeed");
    store
        .close_session("alice", &session.id)
        .await
        .expect("closing the session should succeed");

    let error = client
        .request_reply(pending.turn_handle())
        .await
        .expect_err("closed sessions should surface turn runtime failures");

    assert!(matches!(
        error,
        MockClientError::TurnRuntime { message } if message == "session already closed"
    ));

    let _ = shutdown_tx.send(());
}
