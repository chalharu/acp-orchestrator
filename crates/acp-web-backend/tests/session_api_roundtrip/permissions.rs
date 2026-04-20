use super::support::*;
use acp_contracts::ResolvePermissionRequest;
use acp_mock::MANUAL_PERMISSION_TRIGGER;
use std::time::Duration;

struct PendingPermissionFlow {
    stack: TestStack,
    session_id: String,
    events: SseStream,
}

#[tokio::test]
async fn permission_requests_can_be_approved_through_http() -> Result<()> {
    let mut flow = start_pending_permission_flow().await?;
    let request = next_permission_request(&mut flow.events).await?;
    assert_eq!(request.request_id, "req_1");
    assert_eq!(request.summary, "read_text_file README.md");

    let resolution = flow
        .stack
        .resolve_permission(
            "alice",
            &flow.session_id,
            &request.request_id,
            PermissionDecision::Approve,
        )
        .await?;
    assert_eq!(resolution.request_id, "req_1");

    assert_snapshot_without_pending_permissions(expect_next_event(&mut flow.events).await?);
    assert_assistant_message(expect_next_event(&mut flow.events).await?);

    Ok(())
}

#[tokio::test]
async fn permission_requests_can_be_denied_without_recording_an_assistant_reply() -> Result<()> {
    let mut flow = start_pending_permission_flow().await?;
    let request = next_permission_request(&mut flow.events).await?;

    let resolution = flow
        .stack
        .resolve_permission(
            "alice",
            &flow.session_id,
            &request.request_id,
            PermissionDecision::Deny,
        )
        .await?;
    assert_eq!(resolution.request_id, "req_1");
    assert_snapshot_without_pending_permissions(expect_next_event(&mut flow.events).await?);
    sleep(Duration::from_millis(100)).await;

    let history = flow
        .stack
        .session_history("alice", &flow.session_id)
        .await?;
    assert_eq!(history.messages.len(), 1);
    assert_eq!(history.messages[0].text, MANUAL_PERMISSION_TRIGGER);

    Ok(())
}

#[tokio::test]
async fn session_snapshot_replays_pending_permissions_for_attach() -> Result<()> {
    let mut flow = start_pending_permission_flow().await?;
    let request = next_permission_request(&mut flow.events).await?;

    let snapshot = flow
        .stack
        .session_snapshot("alice", &flow.session_id)
        .await?;
    assert_eq!(snapshot.session.pending_permissions, vec![request]);

    Ok(())
}

#[tokio::test]
async fn cancelling_a_pending_permission_turn_returns_a_status_event() -> Result<()> {
    let mut flow = start_pending_permission_flow().await?;
    let _ = next_permission_request(&mut flow.events).await?;

    let cancelled = flow.stack.cancel_turn("alice", &flow.session_id).await?;
    assert!(cancelled.cancelled);

    assert_snapshot_without_pending_permissions(expect_next_event(&mut flow.events).await?);
    assert_cancelled_status(expect_next_event(&mut flow.events).await?);

    Ok(())
}

#[tokio::test]
async fn resolving_unknown_permission_requests_returns_not_found() -> Result<()> {
    let stack = TestStack::spawn(ServerConfig {
        session_cap: 8,
        acp_server: String::new(),
        startup_hints: false,
        state_dir: test_state_dir(),
        frontend_dist: None,
    })
    .await?;
    let session = stack.create_session("alice").await?;

    let response = stack
        .client
        .post(format!(
            "{}/api/v1/sessions/{}/permissions/req_missing",
            stack.backend_url, session.session.id
        ))
        .bearer_auth("alice")
        .json(&ResolvePermissionRequest {
            decision: PermissionDecision::Approve,
        })
        .send()
        .await
        .context("resolving a missing permission request")?;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    Ok(())
}

async fn start_pending_permission_flow() -> Result<PendingPermissionFlow> {
    let stack = TestStack::spawn(ServerConfig {
        session_cap: 8,
        acp_server: String::new(),
        startup_hints: false,
        state_dir: test_state_dir(),
        frontend_dist: None,
    })
    .await?;
    let session = stack.create_session("alice").await?;
    let session_id = session.session.id.clone();
    let mut events = stack.open_events("alice", &session_id).await?;

    assert_snapshot(expect_next_event(&mut events).await?);
    stack
        .submit_prompt("alice", &session_id, MANUAL_PERMISSION_TRIGGER)
        .await?;
    assert_user_message(expect_next_event(&mut events).await?);

    Ok(PendingPermissionFlow {
        stack,
        session_id,
        events,
    })
}

async fn next_permission_request(
    events: &mut SseStream,
) -> Result<acp_contracts::PermissionRequest> {
    match expect_next_event(events).await?.payload {
        StreamEventPayload::PermissionRequested { request } => Ok(request),
        payload => panic!("unexpected payload: {payload:?}"),
    }
}

fn assert_snapshot(event: StreamEvent) {
    assert!(matches!(
        event.payload,
        StreamEventPayload::SessionSnapshot { .. }
    ));
}

fn assert_snapshot_without_pending_permissions(event: StreamEvent) {
    assert!(matches!(
        event.payload,
        StreamEventPayload::SessionSnapshot { session } if session.pending_permissions.is_empty()
    ));
}

fn assert_user_message(event: StreamEvent) {
    assert!(matches!(
        event.payload,
        StreamEventPayload::ConversationMessage { message }
            if matches!(message.role, MessageRole::User)
    ));
}

fn assert_assistant_message(event: StreamEvent) {
    assert!(matches!(
        event.payload,
        StreamEventPayload::ConversationMessage { message }
            if matches!(message.role, MessageRole::Assistant)
                && message.text.starts_with("mock assistant:")
    ));
}

fn assert_cancelled_status(event: StreamEvent) {
    assert!(matches!(
        event.payload,
        StreamEventPayload::Status { message } if message == "turn cancelled"
    ));
}
