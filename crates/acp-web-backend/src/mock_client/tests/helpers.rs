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
