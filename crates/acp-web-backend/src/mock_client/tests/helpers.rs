use super::*;

#[test]
fn reply_from_stop_reason_maps_cancelled_and_empty_outputs() {
    assert_eq!(
        reply_from_stop_reason(acp::StopReason::Cancelled, "ignored".to_string()),
        ReplyResult::Status("turn cancelled".to_string())
    );
    assert_eq!(
        reply_from_stop_reason(acp::StopReason::EndTurn, String::new()),
        ReplyResult::NoOutput
    );
    assert_eq!(
        reply_from_stop_reason(acp::StopReason::EndTurn, "reply".to_string()),
        ReplyResult::Reply("reply".to_string())
    );
}

#[tokio::test]
async fn cancelled_before_prompt_aborts_pending_io_tasks() {
    let started = Arc::new(Notify::new());
    let started_task = started.clone();
    let io_task = pending_io_task!(started_task);
    started.notified().await;

    assert_eq!(
        cancelled_before_prompt(io_task).await,
        ReplyResult::Status("turn cancelled".to_string())
    );
}

#[tokio::test]
async fn cancelled_before_prompt_reply_returns_none_when_the_turn_is_not_cancelled() {
    let mut io_task = Some(tokio::spawn(async {}));
    let (_cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);

    assert_eq!(
        cancelled_before_prompt_reply(&cancel_rx, &mut io_task).await,
        None
    );
    let _ = io_task.expect("task should remain available").await;
}

#[tokio::test]
async fn cancelled_before_prompt_reply_returns_a_status_when_the_turn_is_cancelled() {
    let started = Arc::new(Notify::new());
    let started_task = started.clone();
    let mut io_task = Some(pending_io_task!(started_task));
    started.notified().await;
    let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
    let _ = cancel_tx.send(true);

    assert_eq!(
        cancelled_before_prompt_reply(&cancel_rx, &mut io_task).await,
        Some(ReplyResult::Status("turn cancelled".to_string()))
    );
    assert!(io_task.is_none());
}

#[test]
fn reuse_cached_session_keeps_successful_session_mappings() {
    let mut upstream_sessions = HashMap::from([("backend".to_string(), "mock_0".to_string())]);

    assert_eq!(
        reuse_cached_session(
            Some("mock_0".to_string()),
            true,
            &mut upstream_sessions,
            "backend",
        ),
        Some("mock_0".to_string())
    );
    assert_eq!(
        upstream_sessions.get("backend").map(String::as_str),
        Some("mock_0")
    );
}

#[test]
fn reuse_cached_session_clears_stale_mappings_after_failed_loads() {
    let mut upstream_sessions = HashMap::from([("backend".to_string(), "mock_0".to_string())]);

    assert_eq!(
        reuse_cached_session(
            Some("mock_0".to_string()),
            false,
            &mut upstream_sessions,
            "backend",
        ),
        None
    );
    assert!(
        !upstream_sessions.contains_key("backend"),
        "stale cached mappings should be removed"
    );
}
