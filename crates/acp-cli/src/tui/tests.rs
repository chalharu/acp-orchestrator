use super::{app::ChatApp, render};
use acp_contracts::{CompletionCandidate, CompletionKind, MessageRole, PermissionRequest};
use chrono::Utc;
use ratatui::{Terminal, backend::TestBackend, layout::Rect};

use crate::events::StreamUpdate;

fn command_candidate(label: &str, detail: &str) -> CompletionCandidate {
    CompletionCandidate {
        label: label.to_string(),
        insert_text: label.to_string(),
        detail: detail.to_string(),
        kind: CompletionKind::Command,
    }
}

fn assistant_message(id: &str, text: &str) -> acp_contracts::ConversationMessage {
    acp_contracts::ConversationMessage {
        id: id.to_string(),
        role: MessageRole::Assistant,
        text: text.to_string(),
        created_at: Utc::now(),
    }
}

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
}

#[test]
fn composer_cursor_position_uses_display_width_for_unicode_input() {
    let mut app = ChatApp::new("s_test", "http://127.0.0.1:8080", false, &[], &[], vec![]);
    app.insert_char('é');
    app.insert_char('界');

    assert_eq!(app.cursor(), 5);
    assert_eq!(app.cursor_display_width(), 3);
    assert_eq!(
        render::composer_cursor_position(Rect::new(0, 0, 20, 3), &app),
        (4, 1)
    );
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
fn render_draws_the_slice5_panes() {
    let backend = TestBackend::new(120, 30);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
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

    terminal
        .draw(|frame| render::render(frame, &app))
        .expect("drawing the slice5 UI should succeed");

    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.contains("Session / Commands"));
    assert!(rendered.contains("Transcript (follow)"));
    assert!(rendered.contains("Tool / Status"));
    assert!(rendered.contains("Composer"));
}

#[test]
fn render_draws_the_completion_menu() {
    let backend = TestBackend::new(120, 30);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
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

    terminal
        .draw(|frame| render::render(frame, &app))
        .expect("drawing completion menu should succeed");

    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.contains("Slash Completion"));
    assert!(rendered.contains("/approve <request-id>"));
}
