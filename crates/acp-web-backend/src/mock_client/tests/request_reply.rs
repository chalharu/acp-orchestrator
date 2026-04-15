use super::*;
use crate::sessions::TurnHandle;

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
async fn prime_session_hint_collects_startup_hints_without_polluting_prompt_replies() {
    let (mock_address, shutdown_tx) = spawn_mock_server_with_config(MockConfig {
        response_delay: Duration::from_millis(1),
        startup_hints: true,
    })
    .await;
    let client = MockClient::new(mock_address).expect("client construction should succeed");
    let store = SessionStore::new(4);
    let session = store
        .create_session("alice")
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
async fn request_reply_reuses_upstream_sessions_for_the_same_backend_session() {
    let (mock_address, shutdown_tx) = spawn_mock_server(Duration::from_millis(1)).await;
    let client = MockClient::new(mock_address).expect("client construction should succeed");
    let store = SessionStore::new(4);
    let session = store
        .create_session("alice")
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
        .create_session("bob")
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
        .create_session("alice")
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
    let client = MockClient::with_timeout("127.0.0.1:9".to_string(), Duration::from_millis(20))
        .expect("client construction should succeed");
    let pending = test_pending_prompt("alice", "hello").await;

    let error = client
        .request_reply(pending.turn_handle())
        .await
        .expect_err("unreachable mock transports should fail");

    assert!(matches!(error, MockClientError::Connect { .. }));
}

#[tokio::test]
async fn request_reply_returns_cancelled_status_when_turns_are_cancelled() {
    let (mock_address, shutdown_tx) = spawn_mock_server(Duration::from_millis(200)).await;
    let client = MockClient::new(mock_address).expect("client construction should succeed");
    let store = SessionStore::new(4);
    let session = store
        .create_session("alice")
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
        .create_session("alice")
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
        .create_session("alice")
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
