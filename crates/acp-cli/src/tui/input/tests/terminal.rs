use super::*;

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
async fn handle_terminal_event_routes_tab_keys_through_the_viewport() {
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
        app
    })
    .await
    .expect("event worker should join");

    assert!(app.completion_menu().is_some());
    assert_eq!(
        request_line_rx
            .await
            .expect("request line should be captured"),
        "GET /api/v1/completions/slash?sessionId=s_test&prefix=%2F HTTP/1.1"
    );
}

#[test]
fn handle_terminal_event_routes_enter_keys_through_the_viewport() {
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
    let terminal_size = ratatui::layout::Size {
        width: 80,
        height: 24,
    };
    let mut app = base_app();
    app.show_completion_menu(vec![
        command_candidate("/help", "/help", "Show help"),
        command_candidate("/quit", "/quit", "Quit"),
    ]);

    handle_terminal_event(
        terminal_size,
        &context,
        &mut app,
        Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
    )
    .expect("enter should apply the selected completion");

    assert_eq!(app.input(), "/help");
}
