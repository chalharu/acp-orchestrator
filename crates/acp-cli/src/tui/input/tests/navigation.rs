use super::*;

fn navigation_app() -> ChatApp {
    ChatApp::new(
        "s_test",
        "http://127.0.0.1:8080",
        false,
        &[acp_contracts::ConversationMessage {
            id: "m_1".to_string(),
            role: acp_contracts::MessageRole::Assistant,
            text: "line 1\nline 2\nline 3".to_string(),
            created_at: chrono::Utc::now(),
        }],
        &[],
        vec![],
    )
}

fn history_navigation_app() -> ChatApp {
    let mut app = navigation_app();
    app.record_submitted_input("first");
    app.record_submitted_input("second");
    app
}

fn set_input(app: &mut ChatApp, value: &str) {
    for character in value.chars() {
        app.insert_char(character);
    }
}

#[test]
fn handle_key_recalls_recent_history_with_up() {
    let runtime = tokio::runtime::Runtime::new().expect("runtime should build");
    let client = Client::builder().build().expect("client should build");
    let context = build_context(
        runtime.handle(),
        &client,
        "http://127.0.0.1:9",
        "developer",
        "s_test",
    );
    let mut app = history_navigation_app();

    handle_key(
        &context,
        &mut app,
        KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
        3,
        8,
    )
    .expect("up should succeed");
    assert_eq!(app.input(), "second");
    handle_key(
        &context,
        &mut app,
        KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
        3,
        8,
    )
    .expect("second up should succeed");
    assert_eq!(app.input(), "first");
}

#[test]
fn handle_key_restores_draft_after_history_navigation() {
    let runtime = tokio::runtime::Runtime::new().expect("runtime should build");
    let client = Client::builder().build().expect("client should build");
    let context = build_context(
        runtime.handle(),
        &client,
        "http://127.0.0.1:9",
        "developer",
        "s_test",
    );
    let mut app = history_navigation_app();
    set_input(&mut app, "draft");
    handle_key(
        &context,
        &mut app,
        KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
        3,
        8,
    )
    .expect("up should succeed");
    handle_key(
        &context,
        &mut app,
        KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
        3,
        8,
    )
    .expect("second down should succeed");

    assert!(app.follow_transcript());
    assert_eq!(app.input(), "draft");
}

#[test]
fn handle_key_arrow_keys_leave_transcript_follow_mode_alone_without_history() {
    let runtime = tokio::runtime::Runtime::new().expect("runtime should build");
    let client = Client::builder().build().expect("client should build");
    let server_url = "http://127.0.0.1:9".to_string();
    let auth_token = "developer".to_string();
    let session_id = "s_test".to_string();
    let context = build_context(
        runtime.handle(),
        &client,
        &server_url,
        &auth_token,
        &session_id,
    );
    let mut app = base_app();

    handle_key(
        &context,
        &mut app,
        KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
        10,
        80,
    )
    .expect("up should succeed");
    handle_key(
        &context,
        &mut app,
        KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
        10,
        80,
    )
    .expect("down should succeed");

    assert!(app.follow_transcript());
}

#[test]
fn handle_key_scrolls_with_page_keys_and_end() {
    let runtime = tokio::runtime::Runtime::new().expect("runtime should build");
    let client = Client::builder().build().expect("client should build");
    let server_url = "http://127.0.0.1:9".to_string();
    let auth_token = "developer".to_string();
    let session_id = "s_test".to_string();
    let context = build_context(
        runtime.handle(),
        &client,
        &server_url,
        &auth_token,
        &session_id,
    );
    let mut app = navigation_app();

    handle_key(
        &context,
        &mut app,
        KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE),
        3,
        8,
    )
    .expect("page up should succeed");
    handle_key(
        &context,
        &mut app,
        KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
        3,
        8,
    )
    .expect("page down should succeed");
    handle_key(
        &context,
        &mut app,
        KeyEvent::new(KeyCode::End, KeyModifiers::NONE),
        3,
        8,
    )
    .expect("end should succeed");

    assert!(app.follow_transcript());
}

#[test]
fn handle_key_ignores_unsupported_navigation_keys() {
    let runtime = tokio::runtime::Runtime::new().expect("runtime should build");
    let client = Client::builder().build().expect("client should build");
    let server_url = "http://127.0.0.1:9".to_string();
    let auth_token = "developer".to_string();
    let session_id = "s_test".to_string();
    let context = build_context(
        runtime.handle(),
        &client,
        &server_url,
        &auth_token,
        &session_id,
    );
    let mut app = navigation_app();

    handle_key(
        &context,
        &mut app,
        KeyEvent::new(KeyCode::Home, KeyModifiers::NONE),
        3,
        8,
    )
    .expect("unsupported keys should be ignored");

    assert_eq!(app.transcript_start(3, 8), 2);
}
