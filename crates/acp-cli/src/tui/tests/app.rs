use super::*;

#[test]
fn chat_app_tracks_follow_and_manual_scroll_modes() {
    let messages = (0..6)
        .map(|index| assistant_message(&format!("m_{index}"), &format!("message {index}")))
        .collect::<Vec<_>>();
    let mut app = ChatApp::new(
        "s_test",
        "http://127.0.0.1:8080",
        false,
        &messages,
        &[],
        vec![command_candidate("/help", "Show available slash commands")],
    );

    assert!(app.follow_transcript());
    assert_eq!(app.transcript_start(3, 40), 3);

    app.scroll_up(3, 40, 1);
    assert!(!app.follow_transcript());
    assert_eq!(app.transcript_start(3, 40), 2);

    app.scroll_down(3, 40, 1);
    assert!(app.follow_transcript());
    assert_eq!(app.transcript_start(3, 40), 3);
}

#[test]
fn chat_app_updates_pending_permissions_and_connection_state() {
    let mut app = ChatApp::new(
        "s_test",
        "http://127.0.0.1:8080",
        true,
        &[assistant_message("m_1", "hello")],
        &[],
        vec![],
    );

    app.apply_stream_update(StreamUpdate::PermissionRequested(PermissionRequest {
        request_id: "req_1".to_string(),
        summary: "read_text_file README.md".to_string(),
    }));
    app.apply_stream_update(StreamUpdate::Status("working".to_string()));
    app.apply_stream_update(StreamUpdate::SessionClosed {
        session_id: "s_test".to_string(),
        reason: "done".to_string(),
    });

    assert_eq!(app.pending_permissions().len(), 1);
    assert!(
        app.status_entries()
            .iter()
            .any(|status| status == "working")
    );
    assert_eq!(app.connection().label(), "closed");
    assert_eq!(app.connection().detail(), Some("done"));
}

#[test]
fn chat_app_scrolls_wrapped_transcript_rows() {
    let message = assistant_message("m_1", "1234567890123456789012345678");
    let mut app = ChatApp::new(
        "s_test",
        "http://127.0.0.1:8080",
        false,
        &[message],
        &[],
        vec![],
    );

    assert_eq!(app.transcript_start(3, 8), 2);
    app.scroll_up(3, 8, 1);
    assert!(!app.follow_transcript());
    assert_eq!(app.transcript_start(3, 8), 1);

    app.scroll_down(3, 8, 1);
    assert!(app.follow_transcript());
    assert_eq!(app.transcript_start(3, 8), 2);
}

#[test]
fn chat_app_manages_editor_state_and_permission_updates() {
    let mut app = ChatApp::new(
        "s_test",
        "http://127.0.0.1:8080",
        false,
        &[],
        &[PermissionRequest {
            request_id: "req_1".to_string(),
            summary: "old request".to_string(),
        }],
        vec![command_candidate("/help", "Show available slash commands")],
    );

    app.insert_char('a');
    app.insert_char('b');
    app.move_cursor_left();
    app.show_completion_menu(vec![
        command_candidate("/help", "Show available slash commands"),
        CompletionCandidate {
            label: "/approve <request-id>".to_string(),
            insert_text: "/approve ".to_string(),
            detail: "Approve a pending permission request".to_string(),
            kind: CompletionKind::Command,
        },
    ]);
    app.select_next_completion();
    app.apply_selected_completion();
    app.clear_input();
    app.backspace();
    app.move_cursor_right();
    app.request_quit();
    app.replace_pending_permissions(vec![PermissionRequest {
        request_id: "req_2".to_string(),
        summary: "new request".to_string(),
    }]);
    app.remove_pending_permission("req_missing");
    app.remove_pending_permission("req_2");
    app.set_command_catalog(vec![command_candidate("/quit", "Exit chat")]);
    for index in 0..40 {
        app.push_status(format!("status {index}"));
    }

    assert!(app.should_quit());
    assert!(app.pending_permissions().is_empty());
    assert_eq!(app.command_catalog()[0].label, "/quit");
    assert_eq!(app.status_entries().len(), 32);
    assert_eq!(app.status_entries()[0], "status 8");
}

#[test]
fn chat_app_formats_user_messages_and_connection_details() {
    let mut app = ChatApp::new(
        "s_test",
        "http://127.0.0.1:8080",
        false,
        &[user_message("m_1", "hello\ncontinued")],
        &[],
        vec![],
    );
    app.set_connection_lost("stream closed");

    assert_eq!(app.transcript()[0], "[user] hello");
    assert_eq!(app.transcript()[1], "  continued");
    assert_eq!(app.connection().label(), "disconnected");
    assert_eq!(app.connection().detail(), Some("stream closed"));
}

#[test]
fn chat_app_recalls_submitted_inputs_and_restores_drafts() {
    let mut app = ChatApp::new("s_test", "http://127.0.0.1:8080", false, &[], &[], vec![]);
    app.record_submitted_input("first");
    app.record_submitted_input("second");
    for value in "draft".chars() {
        app.insert_char(value);
    }

    app.recall_previous_input();
    assert_eq!(app.input(), "second");
    app.recall_previous_input();
    assert_eq!(app.input(), "first");
    app.recall_next_input();
    assert_eq!(app.input(), "second");
    app.recall_next_input();
    assert_eq!(app.input(), "draft");
}

#[test]
fn chat_app_keeps_follow_mode_when_the_transcript_fits_the_viewport() {
    let mut app = ChatApp::new(
        "s_test",
        "http://127.0.0.1:8080",
        false,
        &[assistant_message("m_1", "hello")],
        &[],
        vec![],
    );

    app.scroll_up(10, 80, 1);

    assert!(app.follow_transcript());
    assert_eq!(app.transcript_start(10, 80), 0);
}

#[test]
fn chat_app_handles_noop_paths_and_duplicate_permission_events() {
    let mut app = ChatApp::new("s_test", "http://127.0.0.1:8080", false, &[], &[], vec![]);

    app.move_cursor_left();
    app.move_cursor_right();
    app.select_next_completion();
    app.select_previous_completion();
    app.apply_selected_completion();
    app.scroll_down(3, 8, 1);
    app.apply_stream_update(StreamUpdate::ConversationMessage(assistant_message(
        "m_1",
        "follow-up",
    )));
    app.apply_stream_update(StreamUpdate::PermissionRequested(PermissionRequest {
        request_id: "req_1".to_string(),
        summary: "read_text_file README.md".to_string(),
    }));
    app.apply_stream_update(StreamUpdate::PermissionRequested(PermissionRequest {
        request_id: "req_1".to_string(),
        summary: "read_text_file README.md".to_string(),
    }));

    assert_eq!(app.connection().detail(), None);
    assert!(app.follow_transcript());
    assert_eq!(app.transcript()[0], "[assistant] follow-up");
    assert_eq!(app.pending_permissions().len(), 1);
}

#[test]
fn chat_app_scroll_helpers_cover_follow_boundary_transitions() {
    let messages = (0..6)
        .map(|index| assistant_message(&format!("m_{index}"), &format!("message {index}")))
        .collect::<Vec<_>>();
    let mut app = ChatApp::new(
        "s_test",
        "http://127.0.0.1:8080",
        false,
        &messages,
        &[],
        vec![],
    );

    app.scroll_up(3, 40, 1);
    assert!(!app.follow_transcript());

    app.scroll_down(3, 40, 10);
    assert!(app.follow_transcript());
}
