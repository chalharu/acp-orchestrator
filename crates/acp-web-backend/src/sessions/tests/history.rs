use super::*;

async fn next_event(receiver: &mut broadcast::Receiver<StreamEvent>, message: &str) -> StreamEvent {
    receiver.recv().await.expect(message)
}

async fn assert_no_follow_up(receiver: &mut broadcast::Receiver<StreamEvent>, message: &str) {
    let no_follow_up =
        tokio::time::timeout(std::time::Duration::from_millis(100), receiver.recv()).await;
    assert!(no_follow_up.is_err(), "{message}");
}

#[tokio::test]
async fn session_history_includes_completed_replies() {
    let store = SessionStore::new(4);
    let session = store
        .create_session("alice")
        .await
        .expect("session creation should succeed");
    let pending = store
        .submit_prompt("alice", &session.id, "hello".to_string())
        .await
        .expect("prompt submission should succeed");

    pending.complete_with_reply("hello back".to_string()).await;

    let history = store
        .session_history("alice", &session.id)
        .await
        .expect("session history should load");

    assert_eq!(history.len(), 2);
    assert!(matches!(history[0].role, MessageRole::User));
    assert_eq!(history[0].text, "hello");
    assert!(matches!(history[1].role, MessageRole::Assistant));
    assert_eq!(history[1].text, "hello back");
}

#[tokio::test]
async fn pending_prompts_expose_session_prompt_and_turn_handles() {
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

    assert_eq!(pending.session_id(), session.id);
    assert_eq!(pending.prompt_text(), "hello");
    assert_eq!(turn.session_id(), session.id);
    assert_eq!(turn.prompt_text(), "hello");
}

#[tokio::test]
async fn assistant_replies_follow_prompt_submission_order() {
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

    second
        .complete_with_reply("reply for second".to_string())
        .await;

    let history = store
        .session_history("alice", &session.id)
        .await
        .expect("session history should load");
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].text, "first");
    assert_eq!(history[1].text, "second");

    first
        .complete_with_reply("reply for first".to_string())
        .await;

    let history = store
        .session_history("alice", &session.id)
        .await
        .expect("session history should load");
    assert_eq!(history.len(), 4);
    assert!(matches!(history[2].role, MessageRole::Assistant));
    assert_eq!(history[2].text, "reply for first");
    assert!(matches!(history[3].role, MessageRole::Assistant));
    assert_eq!(history[3].text, "reply for second");
}

#[tokio::test]
async fn pending_permission_resolutions_can_default_to_cancelled() {
    assert_eq!(
        PendingPermissionResolution::cancelled().wait().await,
        PermissionResolutionOutcome::Cancelled
    );
}

#[tokio::test]
async fn complete_without_output_releases_queued_follow_up_events() {
    let store = SessionStore::new(4);
    let session = store
        .create_session("alice")
        .await
        .expect("session creation should succeed");
    let (_snapshot, mut receiver) = store
        .session_events("alice", &session.id)
        .await
        .expect("subscribing should succeed");
    let first = store
        .submit_prompt("alice", &session.id, "first".to_string())
        .await
        .expect("first prompt submission should succeed");
    let second = store
        .submit_prompt("alice", &session.id, "second".to_string())
        .await
        .expect("second prompt submission should succeed");

    let _ = next_event(&mut receiver, "first user event should arrive").await;
    let _ = next_event(&mut receiver, "second user event should arrive").await;

    second
        .complete_with_reply("reply for second".to_string())
        .await;
    assert_no_follow_up(
        &mut receiver,
        "later replies should stay queued until earlier prompts complete",
    )
    .await;

    first.complete_without_output().await;

    let assistant_event =
        next_event(&mut receiver, "queued assistant reply should be broadcast").await;
    assert!(matches!(
        assistant_event.payload,
        StreamEventPayload::ConversationMessage { message }
            if matches!(message.role, MessageRole::Assistant)
                && message.text == "reply for second"
    ));
}

#[tokio::test]
async fn complete_without_output_is_ignored_after_session_close() {
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
        .submit_prompt("alice", &session.id, "hello".to_string())
        .await
        .expect("prompt submission should succeed");

    let _ = next_event(&mut receiver, "user event should arrive").await;
    store
        .close_session("alice", &session.id)
        .await
        .expect("closing the session should succeed");
    let _ = next_event(&mut receiver, "close event should arrive").await;

    pending.complete_without_output().await;

    assert_no_follow_up(
        &mut receiver,
        "closed sessions should ignore pending silent completions",
    )
    .await;
}

#[tokio::test]
async fn pending_replies_are_ignored_after_session_close() {
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
    pending.complete_with_reply("late reply".to_string()).await;

    let history = store
        .session_history("alice", &session.id)
        .await
        .expect("session history should load");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].text, "hello");
}

#[tokio::test]
async fn empty_prompts_are_rejected() {
    let store = SessionStore::new(4);
    let session = store
        .create_session("alice")
        .await
        .expect("session creation should succeed");

    let error = store
        .submit_prompt("alice", &session.id, "   ".to_string())
        .await
        .expect_err("empty prompt should fail");

    assert_eq!(error, SessionStoreError::EmptyPrompt);
    assert_eq!(error.message(), "prompt must not be empty");
}

#[tokio::test]
async fn pending_prompts_can_broadcast_status_updates() {
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
        .submit_prompt("alice", &session.id, "hello".to_string())
        .await
        .expect("prompt submission should succeed");

    let user_event = next_event(&mut receiver, "user event should arrive").await;
    assert!(matches!(
        user_event.payload,
        StreamEventPayload::ConversationMessage { message }
            if matches!(message.role, MessageRole::User)
    ));

    pending.complete_with_status("ACP request failed").await;

    let status_event = next_event(&mut receiver, "status event should arrive").await;
    assert!(matches!(
        status_event.payload,
        StreamEventPayload::Status { message } if message == "ACP request failed"
    ));
}

#[tokio::test]
async fn pending_status_updates_are_ignored_after_session_close() {
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
        .submit_prompt("alice", &session.id, "hello".to_string())
        .await
        .expect("prompt submission should succeed");

    let _ = next_event(&mut receiver, "user event should arrive").await;
    let _ = store
        .close_session("alice", &session.id)
        .await
        .expect("closing should succeed");
    let closed_event = next_event(&mut receiver, "close event should arrive").await;
    assert!(matches!(
        closed_event.payload,
        StreamEventPayload::SessionClosed { .. }
    ));

    pending.complete_with_status("should be ignored").await;

    assert_no_follow_up(&mut receiver, "no extra status event should be broadcast").await;
}
