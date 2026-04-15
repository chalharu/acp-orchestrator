use super::*;

#[test]
fn handle_key_edits_and_navigates_transcript() {
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
    let mut app = ChatApp::new(
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
    );

    handle_key(
        &context,
        &mut app,
        KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE),
        3,
        40,
    )
    .expect("editing should succeed");
    handle_key(
        &context,
        &mut app,
        KeyEvent::new(KeyCode::Left, KeyModifiers::NONE),
        3,
        40,
    )
    .expect("left should succeed");
    handle_key(
        &context,
        &mut app,
        KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
        3,
        40,
    )
    .expect("right should succeed");
    handle_key(
        &context,
        &mut app,
        KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
        3,
        40,
    )
    .expect("backspace should succeed");
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
    .expect("down should succeed");
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
        KeyEvent::new(KeyCode::Home, KeyModifiers::NONE),
        3,
        8,
    )
    .expect("unsupported keys should be ignored");
    handle_key(
        &context,
        &mut app,
        KeyEvent::new(KeyCode::End, KeyModifiers::NONE),
        3,
        8,
    )
    .expect("end should succeed");

    assert!(app.follow_transcript());
    assert_eq!(app.input(), "");
}

#[test]
fn handle_terminal_event_handles_paste_and_ignores_other_events() {
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
    let terminal_size = ratatui::layout::Size {
        width: 80,
        height: 24,
    };

    handle_terminal_event(
        terminal_size,
        &context,
        &mut app,
        Event::Paste("abc".to_string()),
    )
    .expect("paste events should update the composer");
    handle_terminal_event(terminal_size, &context, &mut app, Event::Resize(80, 24))
        .expect("non-input events should be ignored");

    assert_eq!(app.input(), "abc");
}

#[tokio::test]
async fn handle_terminal_event_routes_key_events_through_the_viewport() {
    let (url, request_line_rx) = spawn_completion_server(SlashCompletionsResponse {
        candidates: vec![
            command_candidate("/help", "/help", "Show help"),
            command_candidate("/quit", "/quit", "Quit"),
        ],
    })
    .await;
    let client = Client::builder().build().expect("client should build");
    let auth_token = "developer".to_string();
    let session_id = "s_test".to_string();
    let server_url = url.clone();
    let runtime_handle = Handle::current();

    let app = tokio::task::spawn_blocking(move || {
        let mut app = base_app();
        let terminal_size = ratatui::layout::Size {
            width: 80,
            height: 24,
        };

        app.insert_char('/');
        handle_terminal_event(
            terminal_size,
            &build_context(
                &runtime_handle,
                &client,
                &server_url,
                &auth_token,
                &session_id,
            ),
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
        )
        .expect("tab should load slash completions");
        handle_terminal_event(
            terminal_size,
            &build_context(
                &runtime_handle,
                &client,
                &server_url,
                &auth_token,
                &session_id,
            ),
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        )
        .expect("enter should apply the selected completion");
        app
    })
    .await
    .expect("event worker should join");

    assert_eq!(app.input(), "/help");
    assert_eq!(
        request_line_rx
            .await
            .expect("request line should be captured"),
        "GET /api/v1/completions/slash?sessionId=s_test&prefix=%2F HTTP/1.1"
    );
}
