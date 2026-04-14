use super::*;

async fn register_readme_permission(
    pending: &PendingPrompt,
) -> Result<PendingPermissionResolution, SessionStoreError> {
    register_permission(pending, "read_text_file README.md").await
}

async fn register_permission(
    pending: &PendingPrompt,
    summary: &str,
) -> Result<PendingPermissionResolution, SessionStoreError> {
    pending
        .turn_handle()
        .register_permission_request(
            summary.to_string(),
            "allow_once".to_string(),
            "reject_once".to_string(),
        )
        .await
}

async fn expect_permission_event(receiver: &mut broadcast::Receiver<StreamEvent>) {
    let permission_event = receiver
        .recv()
        .await
        .expect("permission event should arrive");
    assert!(matches!(
        permission_event.payload,
        StreamEventPayload::PermissionRequested { request }
            if request.request_id == "req_1"
                && request.summary == "read_text_file README.md"
    ));
}

#[tokio::test]
async fn permission_requests_can_be_resolved_for_the_active_turn() {
    let store = SessionStore::new(4);
    let session = store
        .create_session("alice")
        .await
        .expect("session creation should succeed");
    let (_snapshot, mut receiver) = store
        .session_events("alice", &session.id)
        .await
        .expect("subscribing should succeed");
    let pending = store
        .submit_prompt("alice", &session.id, "permission please".to_string())
        .await
        .expect("prompt submission should succeed");

    let _ = receiver.recv().await.expect("user event should arrive");
    let _cancel_rx = pending
        .turn_handle()
        .start_turn()
        .await
        .expect("starting the turn should succeed");
    let resolution = register_readme_permission(&pending)
        .await
        .expect("permission registration should succeed");

    expect_permission_event(&mut receiver).await;

    let resolved = store
        .resolve_permission("alice", &session.id, "req_1", PermissionDecision::Approve)
        .await
        .expect("permission resolution should succeed");
    assert_eq!(resolved.request_id, "req_1");
    assert_eq!(resolved.decision, PermissionDecision::Approve);
    assert_eq!(
        resolution.wait().await,
        PermissionResolutionOutcome::Selected("allow_once".to_string())
    );
}

#[tokio::test]
async fn permission_requests_without_active_turns_are_cancelled() {
    let store = SessionStore::new(4);
    let session = store
        .create_session("alice")
        .await
        .expect("session creation should succeed");
    let pending = store
        .submit_prompt("alice", &session.id, "permission please".to_string())
        .await
        .expect("prompt submission should succeed");

    let resolution = register_readme_permission(&pending)
        .await
        .expect("permission registration should not fail");

    assert_eq!(
        resolution.wait().await,
        PermissionResolutionOutcome::Cancelled
    );
}

#[tokio::test]
async fn permission_requests_for_non_active_prompts_are_cancelled() {
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
    let _cancel_rx = first
        .turn_handle()
        .start_turn()
        .await
        .expect("starting the first turn should succeed");

    let resolution = register_readme_permission(&second)
        .await
        .expect("mismatched permission registrations should not fail");

    assert_eq!(
        resolution.wait().await,
        PermissionResolutionOutcome::Cancelled
    );
}

#[tokio::test]
async fn cancelling_the_active_turn_cancels_pending_permissions() {
    let store = SessionStore::new(4);
    let session = store
        .create_session("alice")
        .await
        .expect("session creation should succeed");
    let pending = store
        .submit_prompt("alice", &session.id, "permission please".to_string())
        .await
        .expect("prompt submission should succeed");

    let _cancel_rx = pending
        .turn_handle()
        .start_turn()
        .await
        .expect("starting the turn should succeed");
    let resolution = register_readme_permission(&pending)
        .await
        .expect("permission registration should succeed");

    assert!(
        store
            .cancel_active_turn("alice", &session.id)
            .await
            .expect("cancelling should succeed")
    );
    assert_eq!(
        resolution.wait().await,
        PermissionResolutionOutcome::Cancelled
    );
}

#[tokio::test]
async fn session_snapshots_keep_pending_permissions_in_creation_order() {
    let store = SessionStore::new(4);
    let session = store
        .create_session("alice")
        .await
        .expect("session creation should succeed");
    let pending = store
        .submit_prompt("alice", &session.id, "permission please".to_string())
        .await
        .expect("prompt submission should succeed");

    let _cancel_rx = pending
        .turn_handle()
        .start_turn()
        .await
        .expect("starting the turn should succeed");
    for request_number in 0..12 {
        let _ = register_permission(&pending, &format!("permission {request_number}"))
            .await
            .expect("permission registration should succeed");
    }

    let snapshot = store
        .session_snapshot("alice", &session.id)
        .await
        .expect("snapshot loading should succeed");
    let request_ids = snapshot
        .pending_permissions
        .into_iter()
        .map(|request| request.request_id)
        .collect::<Vec<_>>();

    assert_eq!(
        request_ids,
        (1..=12)
            .map(|request_id| format!("req_{request_id}"))
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn closed_sessions_reject_permission_registration_resolution_and_cancellation() {
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

    let registration_error = register_readme_permission(&pending)
        .await
        .expect_err("closed sessions should reject permission registration");
    assert_eq!(registration_error, SessionStoreError::Closed);

    let resolution_error = store
        .resolve_permission("alice", &session.id, "req_1", PermissionDecision::Approve)
        .await
        .expect_err("closed sessions should reject permission resolution");
    assert_eq!(resolution_error, SessionStoreError::Closed);

    let cancel_error = store
        .cancel_active_turn("alice", &session.id)
        .await
        .expect_err("closed sessions should reject turn cancellation");
    assert_eq!(cancel_error, SessionStoreError::Closed);
}

#[tokio::test]
async fn closing_sessions_cancel_active_turns_and_pending_permissions() {
    let store = SessionStore::new(4);
    let session = store
        .create_session("alice")
        .await
        .expect("session creation should succeed");
    let pending = store
        .submit_prompt("alice", &session.id, "permission please".to_string())
        .await
        .expect("prompt submission should succeed");

    let mut cancel_rx = pending
        .turn_handle()
        .start_turn()
        .await
        .expect("starting the turn should succeed");
    let resolution = register_readme_permission(&pending)
        .await
        .expect("permission registration should succeed");

    store
        .close_session("alice", &session.id)
        .await
        .expect("closing the session should succeed");

    cancel_rx
        .changed()
        .await
        .expect("closing the session should cancel the active turn");
    assert!(*cancel_rx.borrow());
    assert_eq!(
        resolution.wait().await,
        PermissionResolutionOutcome::Cancelled
    );
}

#[tokio::test]
async fn cancelling_without_an_active_turn_reports_false() {
    let store = SessionStore::new(4);
    let session = store
        .create_session("alice")
        .await
        .expect("session creation should succeed");

    assert!(
        !store
            .cancel_active_turn("alice", &session.id)
            .await
            .expect("idle cancellation should succeed")
    );
}

#[tokio::test]
async fn closed_sessions_reject_new_prompts_and_second_closes() {
    let store = SessionStore::new(4);
    let session = store
        .create_session("alice")
        .await
        .expect("session creation should succeed");
    store
        .close_session("alice", &session.id)
        .await
        .expect("closing should succeed");

    let prompt_error = store
        .submit_prompt("alice", &session.id, "hello".to_string())
        .await
        .expect_err("closed sessions should reject prompts");
    assert_eq!(prompt_error, SessionStoreError::Closed);
    assert_eq!(prompt_error.message(), "session already closed");

    let close_error = store
        .close_session("alice", &session.id)
        .await
        .expect_err("closing twice should fail");
    assert_eq!(close_error, SessionStoreError::Closed);
}
