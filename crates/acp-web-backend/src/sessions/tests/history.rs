use super::*;

async fn next_event(receiver: &mut broadcast::Receiver<StreamEvent>, message: &str) -> StreamEvent {
    receiver.recv().await.expect(message)
}

async fn assert_no_follow_up(receiver: &mut broadcast::Receiver<StreamEvent>, message: &str) {
    let no_follow_up =
        tokio::time::timeout(std::time::Duration::from_millis(100), receiver.recv()).await;
    assert!(no_follow_up.is_err(), "{message}");
}

async fn started_streaming_turn() -> (
    SessionStore,
    SessionSnapshot,
    broadcast::Receiver<StreamEvent>,
    PendingPrompt,
    TurnHandle,
) {
    let store = SessionStore::new(4);
    let session = store
        .create_session("alice", "w_test")
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
    let turn = pending.turn_handle();
    let _cancel_rx = turn.start_turn().await.expect("turn start should succeed");
    let _ = next_event(&mut receiver, "user event should arrive").await;
    (store, session, receiver, pending, turn)
}

async fn first_streamed_assistant_message_id(
    turn: &TurnHandle,
    receiver: &mut broadcast::Receiver<StreamEvent>,
) -> String {
    turn.stream_assistant_chunk("hel".to_string())
        .await
        .expect("first chunk should stream");
    match next_event(receiver, "first chunk should arrive")
        .await
        .payload
    {
        StreamEventPayload::ConversationMessage { message, partial } => {
            assert!(matches!(message.role, MessageRole::Assistant));
            assert_eq!(message.text, "hel");
            assert!(partial);
            message.id
        }
        other => panic!("unexpected first chunk event: {other:?}"),
    }
}

async fn assert_second_streamed_assistant_chunk(
    turn: &TurnHandle,
    receiver: &mut broadcast::Receiver<StreamEvent>,
    message_id: &str,
) {
    turn.stream_assistant_chunk("lo".to_string())
        .await
        .expect("second chunk should stream");
    let second_chunk = next_event(receiver, "second chunk should arrive").await;
    assert!(matches!(
        second_chunk.payload,
        StreamEventPayload::ConversationMessage { message, partial }
            if message.id == message_id && message.text == "hello" && partial
    ));
}

#[tokio::test]
async fn session_history_includes_completed_replies() {
    let store = SessionStore::new(4);
    let session = store
        .create_session("alice", "w_test")
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
async fn streamed_assistant_chunks_update_one_message_until_completion() {
    let (store, session, mut receiver, pending, turn) = started_streaming_turn().await;
    let message_id = first_streamed_assistant_message_id(&turn, &mut receiver).await;
    assert_second_streamed_assistant_chunk(&turn, &mut receiver, &message_id).await;

    pending.complete_with_reply("hello".to_string()).await;
    let final_event = next_event(&mut receiver, "final reply marker should arrive").await;
    assert!(matches!(
        final_event.payload,
        StreamEventPayload::ConversationMessage { message, partial }
            if message.id == message_id && message.text == "hello" && !partial
    ));

    let history = store
        .session_history("alice", &session.id)
        .await
        .expect("session history should load");
    assert_eq!(history.len(), 2);
    assert_eq!(history[1].id, message_id);
    assert_eq!(history[1].text, "hello");
}

#[tokio::test]
async fn streamed_assistant_chunks_finalize_without_reply_text() {
    let (store, session, mut receiver, pending, turn) = started_streaming_turn().await;
    let message_id = first_streamed_assistant_message_id(&turn, &mut receiver).await;

    pending.complete_without_output().await;
    let final_event = next_event(&mut receiver, "final reply marker should arrive").await;
    assert!(matches!(
        final_event.payload,
        StreamEventPayload::ConversationMessage { message, partial }
            if message.id == message_id && message.text == "hel" && !partial
    ));

    let history = store
        .session_history("alice", &session.id)
        .await
        .expect("session history should load");
    assert_eq!(history[1].id, message_id);
    assert_eq!(history[1].text, "hel");
}

#[tokio::test]
async fn streamed_assistant_chunks_finalize_before_status() {
    let (store, session, mut receiver, pending, turn) = started_streaming_turn().await;
    let message_id = first_streamed_assistant_message_id(&turn, &mut receiver).await;

    pending.complete_with_status("turn cancelled").await;
    let final_event = next_event(&mut receiver, "final reply marker should arrive").await;
    assert!(matches!(
        final_event.payload,
        StreamEventPayload::ConversationMessage { message, partial }
            if message.id == message_id && message.text == "hel" && !partial
    ));
    let status_event = next_event(&mut receiver, "status event should arrive").await;
    assert!(matches!(
        status_event.payload,
        StreamEventPayload::Status { message } if message == "turn cancelled"
    ));

    let history = store
        .session_history("alice", &session.id)
        .await
        .expect("session history should load");
    assert_eq!(history[1].id, message_id);
    assert_eq!(history[1].text, "hel");
}

#[tokio::test]
async fn empty_streamed_assistant_chunks_are_ignored() {
    let (_store, _session, mut receiver, _pending, turn) = started_streaming_turn().await;

    turn.stream_assistant_chunk(String::new())
        .await
        .expect("empty chunks should be accepted");

    assert_no_follow_up(&mut receiver, "empty chunks should not emit events").await;
}

#[tokio::test]
async fn streamed_assistant_chunks_fail_after_session_close() {
    let (store, session, _receiver, _pending, turn) = started_streaming_turn().await;
    store
        .close_session("alice", &session.id)
        .await
        .expect("session close should succeed");

    assert_eq!(
        turn.stream_assistant_chunk("late".to_string()).await,
        Err(SessionStoreError::Closed)
    );
}

#[tokio::test]
async fn final_reply_creates_fallback_when_streamed_message_is_missing() {
    let (store, session, mut receiver, pending, turn) = started_streaming_turn().await;
    let missing_message_id = first_streamed_assistant_message_id(&turn, &mut receiver).await;
    turn.handle
        .remove_message_for_test(&missing_message_id)
        .await;

    pending.complete_with_reply("fallback".to_string()).await;
    let final_event = next_event(&mut receiver, "fallback reply should arrive").await;
    assert!(matches!(
        final_event.payload,
        StreamEventPayload::ConversationMessage { message, partial }
            if message.id != missing_message_id && message.text == "fallback" && !partial
    ));

    let history = store
        .session_history("alice", &session.id)
        .await
        .expect("session history should load");
    assert_eq!(history[1].text, "fallback");
}

#[tokio::test]
async fn final_reply_can_correct_streamed_assistant_text() {
    let store = SessionStore::new(4);
    let session = store
        .create_session("alice", "w_test")
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
    let turn = pending.turn_handle();
    let _cancel_rx = turn.start_turn().await.expect("turn start should succeed");
    let _ = next_event(&mut receiver, "user event should arrive").await;

    turn.stream_assistant_chunk("draft".to_string())
        .await
        .expect("draft chunk should stream");
    let _ = next_event(&mut receiver, "draft chunk should arrive").await;
    pending.complete_with_reply("final".to_string()).await;

    let final_event = next_event(&mut receiver, "final correction should arrive").await;
    assert!(matches!(
        final_event.payload,
        StreamEventPayload::ConversationMessage { message, partial }
            if matches!(message.role, MessageRole::Assistant)
                && message.text == "final"
                && !partial
    ));
}

#[tokio::test]
async fn pending_prompts_expose_session_prompt_and_turn_handles() {
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

    assert_eq!(pending.session_id(), session.id);
    assert_eq!(pending.prompt_text(), "hello");
    assert_eq!(turn.session_id(), session.id);
    assert_eq!(turn.prompt_text(), "hello");
}

#[tokio::test]
async fn assistant_replies_follow_prompt_submission_order() {
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
        .create_session("alice", "w_test")
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
        StreamEventPayload::ConversationMessage { message, .. }
            if matches!(message.role, MessageRole::Assistant)
                && message.text == "reply for second"
    ));
}

#[tokio::test]
async fn complete_without_output_is_ignored_after_session_close() {
    let store = SessionStore::new(4);
    let session = store
        .create_session("alice", "w_test")
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
    pending.complete_with_reply("late reply".to_string()).await;

    let history = store
        .session_history("alice", &session.id)
        .await
        .expect("session history should load");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].text, "hello");
}

#[tokio::test]
async fn appended_assistant_messages_are_rejected_after_session_close() {
    let store = SessionStore::new(4);
    let session = store
        .create_session("alice", "w_test")
        .await
        .expect("session creation should succeed");
    store
        .close_session("alice", &session.id)
        .await
        .expect("closing the session should succeed");

    let error = store
        .append_assistant_message("alice", &session.id, "late reply".to_string())
        .await
        .expect_err("closed sessions should reject injected assistant messages");

    assert_eq!(error, SessionStoreError::Closed);
}

#[tokio::test]
async fn empty_prompts_are_rejected() {
    let store = SessionStore::new(4);
    let session = store
        .create_session("alice", "w_test")
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
async fn runtime_unavailable_sessions_reject_new_prompts_and_turns() {
    let store = SessionStore::new(4);
    let session = store
        .create_session("alice", "w_test")
        .await
        .expect("session creation should succeed");
    let pending = store
        .submit_prompt("alice", &session.id, "hello".to_string())
        .await
        .expect("initial prompt should be accepted");

    store
        .mark_runtime_unavailable("alice", &session.id, "runtime failed".to_string())
        .await
        .expect("active sessions can be marked runtime-unavailable");

    let prompt_error = store
        .submit_prompt("alice", &session.id, "again".to_string())
        .await
        .expect_err("runtime-unavailable sessions should reject prompts");
    assert_eq!(prompt_error, SessionStoreError::RuntimeUnavailable);
    let turn_error = pending
        .turn_handle()
        .start_turn()
        .await
        .expect_err("runtime-unavailable sessions should reject turns");
    assert_eq!(turn_error, SessionStoreError::RuntimeUnavailable);
}

#[tokio::test]
async fn closed_sessions_cannot_be_marked_runtime_unavailable() {
    let store = SessionStore::new(4);
    let session = store
        .create_session("alice", "w_test")
        .await
        .expect("session creation should succeed");
    store
        .close_session("alice", &session.id)
        .await
        .expect("session close should succeed");

    let error = store
        .mark_runtime_unavailable("alice", &session.id, "runtime failed".to_string())
        .await
        .expect_err("closed sessions should reject runtime-unavailable marking");

    assert_eq!(error, SessionStoreError::Closed);
}

#[tokio::test]
async fn pending_prompts_can_broadcast_status_updates() {
    let store = SessionStore::new(4);
    let session = store
        .create_session("alice", "w_test")
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
        StreamEventPayload::ConversationMessage { message, .. }
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
        .create_session("alice", "w_test")
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
