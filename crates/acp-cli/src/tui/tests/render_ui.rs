use super::*;

#[test]
fn render_draws_the_slice5_panes() {
    let app = ChatApp::new(
        "s_test",
        "http://127.0.0.1:8080",
        false,
        &[assistant_message("m_1", "hello from tui")],
        &[PermissionRequest {
            request_id: "req_1".to_string(),
            summary: "read_text_file README.md".to_string(),
        }],
        vec![
            command_candidate("/help", "Show available slash commands"),
            command_candidate("/quit", "Exit chat"),
        ],
    );

    let rendered = rendered_screen(&app);
    assert!(rendered.contains("Session / Commands"));
    assert!(rendered.contains("Transcript (follow)"));
    assert!(rendered.contains("Tool / Status"));
    assert!(rendered.contains("Composer"));
}

#[test]
fn render_draws_the_completion_menu() {
    let mut app = ChatApp::new(
        "s_test",
        "http://127.0.0.1:8080",
        false,
        &[],
        &[],
        vec![command_candidate("/help", "Show available slash commands")],
    );
    app.show_completion_menu(vec![
        command_candidate("/help", "Show available slash commands"),
        CompletionCandidate {
            label: "/approve <request-id>".to_string(),
            insert_text: "/approve ".to_string(),
            detail: "Approve a pending permission request".to_string(),
            kind: CompletionKind::Command,
        },
    ]);

    let rendered = rendered_screen(&app);
    assert!(rendered.contains("Slash Completion"));
    assert!(rendered.contains("/approve <request-id>"));
}

#[test]
fn render_draws_disconnected_empty_states() {
    let mut app = ChatApp::new("s_test", "http://127.0.0.1:8080", false, &[], &[], vec![]);
    app.set_connection_lost("network dropped");
    app.resume_follow();

    let rendered = rendered_screen(&app);
    assert!(rendered.contains("detail: network dropped"));
    assert!(rendered.contains("/help unavailable"));
    assert!(rendered.contains("Transcript (follow)"));
    assert!(rendered.contains("No conversation messages yet."));
    assert!(rendered.contains("pending permissions"));
    assert!(rendered.contains("none"));
}

#[test]
fn render_draws_manual_mode_with_empty_recent_status() {
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
    app.clear_status_entries();
    app.scroll_up(3, 40, 1);

    let rendered = rendered_screen(&app);
    assert!(rendered.contains("Transcript (manual)"));
    assert!(rendered.contains("recent status"));
    assert!(rendered.contains("none"));
}
