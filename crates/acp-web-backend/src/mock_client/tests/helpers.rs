use super::*;
use agent_client_protocol::schema;
use std::collections::HashMap;

#[test]
fn reply_from_stop_reason_maps_cancelled_and_empty_outputs() {
    assert_eq!(
        reply_from_stop_reason(schema::StopReason::Cancelled, "ignored".to_string()),
        ReplyResult::Status("turn cancelled".to_string())
    );
    assert_eq!(
        reply_from_stop_reason(schema::StopReason::EndTurn, String::new()),
        ReplyResult::NoOutput
    );
    assert_eq!(
        reply_from_stop_reason(schema::StopReason::EndTurn, "reply".to_string()),
        ReplyResult::Reply("reply".to_string())
    );
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

#[tokio::test(flavor = "current_thread")]
async fn handle_cancelled_prompt_sends_cancels_and_reports_cancelled_status() {
    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
    let prompt_future = async { Ok(schema::PromptResponse::new(schema::StopReason::EndTurn)) };
    tokio::pin!(prompt_future);
    let client = BackendAcpClient::without_turn();

    let reply = handle_cancelled_prompt(
        true,
        &mut prompt_future,
        schema::CancelNotification::new("mock_0"),
        |cancel_request| {
            cancel_tx
                .send(cancel_request)
                .expect("cancel requests should send");
            Ok(())
        },
        &client,
    )
    .await
    .expect("cancelled prompts should resolve");

    assert_eq!(
        cancel_rx
            .await
            .expect("cancel requests should be received")
            .session_id
            .to_string(),
        "mock_0"
    );
    assert_eq!(reply, ReplyResult::Status("turn cancelled".to_string()));
}
