use super::*;

#[test]
fn completion_query_and_size_to_rect_cover_supported_shapes() {
    assert_eq!(completion_query("/ap", 3), Some("/ap"));
    assert_eq!(completion_query("/approve req_", 13), Some("/approve req_"));
    assert_eq!(completion_query("hello", 5), None);
    assert_eq!(
        size_to_rect(ratatui::layout::Size {
            width: 80,
            height: 24,
        }),
        Rect::new(0, 0, 80, 24)
    );
}

#[test]
fn handle_key_rotates_completion_selection_with_arrows() {
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
    app.show_completion_menu(vec![
        command_candidate("/help", "/help", "Show help"),
        command_candidate("/quit", "/quit", "Quit"),
    ]);

    handle_key(
        &context,
        &mut app,
        KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
        3,
        40,
    )
    .expect("up should rotate completion selection");
    handle_key(
        &context,
        &mut app,
        KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
        3,
        40,
    )
    .expect("down should rotate completion selection");

    assert!(app.completion_menu().is_some());
}

#[test]
fn handle_key_rotates_completion_selection_with_tabs() {
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
    app.show_completion_menu(vec![
        command_candidate("/help", "/help", "Show help"),
        command_candidate("/quit", "/quit", "Quit"),
    ]);

    handle_key(
        &context,
        &mut app,
        KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT),
        3,
        40,
    )
    .expect("backtab should rotate selection");
    handle_key(
        &context,
        &mut app,
        KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
        3,
        40,
    )
    .expect("tab should rotate selection");

    assert!(app.completion_menu().is_some());
}

#[test]
fn handle_key_applies_selected_completion() {
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
    app.show_completion_menu(vec![
        command_candidate("/help", "/help", "Show help"),
        command_candidate("/quit", "/quit", "Quit"),
    ]);

    handle_key(
        &context,
        &mut app,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        3,
        40,
    )
    .expect("enter should apply a completion");

    assert!(app.completion_menu().is_none());
}

#[test]
fn handle_key_clears_menu_and_reports_interrupts() {
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
    app.show_completion_menu(vec![command_candidate("/help", "/help", "Show help")]);

    handle_key(
        &context,
        &mut app,
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
        3,
        40,
    )
    .expect("escape should clear completions");
    handle_key(
        &context,
        &mut app,
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
        3,
        40,
    )
    .expect("ctrl-c should be handled in-process");

    assert!(app.completion_menu().is_none());
    assert!(
        app.status_entries()
            .iter()
            .any(|status| status.contains("interrupted input"))
    );
}

#[tokio::test]
async fn update_completion_menu_loads_candidates_and_auto_applies_single_matches() {
    let (url, request_line_rx) = spawn_completion_server(SlashCompletionsResponse {
        candidates: vec![command_candidate(
            "/approve <request-id>",
            "/approve ",
            "Approve",
        )],
    })
    .await;
    let client = Client::builder().build().expect("client should build");
    let runtime_handle = Handle::current();
    let auth_token = "developer".to_string();
    let session_id = "s_test".to_string();
    let server_url = url.clone();

    let app = tokio::task::spawn_blocking(move || {
        let context = build_context(
            &runtime_handle,
            &client,
            &server_url,
            &auth_token,
            &session_id,
        );
        let mut app = base_app();
        for value in "/ap".chars() {
            app.insert_char(value);
        }
        update_completion_menu(&context, &mut app);
        app
    })
    .await
    .expect("completion worker should join");

    assert_eq!(app.input(), "/approve ");
    assert_eq!(
        request_line_rx
            .await
            .expect("request line should be captured"),
        "GET /api/v1/completions/slash?sessionId=s_test&prefix=%2Fap HTTP/1.1"
    );
}

#[tokio::test]
async fn update_completion_menu_clears_existing_menu_for_non_slash_input() {
    let client = Client::builder().build().expect("client should build");
    let runtime_handle = Handle::current();
    let server_url = "http://127.0.0.1:9".to_string();
    let auth_token = "developer".to_string();
    let session_id = "s_test".to_string();

    let app = tokio::task::spawn_blocking(move || {
        let context = build_context(
            &runtime_handle,
            &client,
            &server_url,
            &auth_token,
            &session_id,
        );
        let mut app = base_app();
        app.show_completion_menu(vec![command_candidate("/help", "/help", "Show help")]);
        for value in "hello".chars() {
            app.insert_char(value);
        }
        update_completion_menu(&context, &mut app);
        app
    })
    .await
    .expect("completion worker should join");

    assert!(app.completion_menu().is_none());
}

#[tokio::test]
async fn update_completion_menu_records_backend_failures() {
    let client = Client::builder().build().expect("client should build");
    let runtime_handle = Handle::current();
    let server_url = "http://127.0.0.1:9".to_string();
    let auth_token = "developer".to_string();
    let session_id = "s_test".to_string();

    let app = tokio::task::spawn_blocking(move || {
        let context = build_context(
            &runtime_handle,
            &client,
            &server_url,
            &auth_token,
            &session_id,
        );
        let mut app = base_app();
        for value in "/ap".chars() {
            app.insert_char(value);
        }
        update_completion_menu(&context, &mut app);
        app
    })
    .await
    .expect("completion worker should join");

    assert!(
        app.status_entries()
            .iter()
            .any(|status| status.starts_with("slash completion unavailable"))
    );
}

#[tokio::test]
async fn update_completion_menu_records_timeouts() {
    let timeout_url = spawn_stalled_server().await;
    let client = Client::builder().build().expect("client should build");
    let runtime_handle = Handle::current();
    let auth_token = "developer".to_string();
    let session_id = "s_test".to_string();
    let timeout_server_url = timeout_url.clone();

    let timed_out_app = tokio::task::spawn_blocking(move || {
        let context = build_context(
            &runtime_handle,
            &client,
            &timeout_server_url,
            &auth_token,
            &session_id,
        );
        let mut app = base_app();
        for value in "/ap".chars() {
            app.insert_char(value);
        }
        update_completion_menu(&context, &mut app);
        app
    })
    .await
    .expect("timeout worker should join");

    assert!(
        timed_out_app
            .status_entries()
            .iter()
            .any(|status| status == "slash completion timed out")
    );
}

#[tokio::test]
async fn update_completion_menu_keeps_multiple_matches_visible() {
    let (url, request_line_rx) = spawn_completion_server(SlashCompletionsResponse {
        candidates: vec![
            command_candidate("/approve <request-id>", "/approve ", "Approve"),
            command_candidate("/deny <request-id>", "/deny ", "Deny"),
        ],
    })
    .await;
    let client = Client::builder().build().expect("client should build");
    let runtime_handle = Handle::current();
    let auth_token = "developer".to_string();
    let session_id = "s_test".to_string();
    let server_url = url.clone();

    let app = tokio::task::spawn_blocking(move || {
        let context = build_context(
            &runtime_handle,
            &client,
            &server_url,
            &auth_token,
            &session_id,
        );
        let mut app = base_app();
        for value in "/".chars() {
            app.insert_char(value);
        }
        update_completion_menu(&context, &mut app);
        app
    })
    .await
    .expect("multi-match worker should join");

    assert_eq!(app.input(), "/");
    assert_eq!(
        app.completion_menu()
            .expect("multiple matches should stay visible")
            .candidates()
            .len(),
        2
    );
    assert_eq!(
        request_line_rx
            .await
            .expect("request line should be captured"),
        "GET /api/v1/completions/slash?sessionId=s_test&prefix=%2F HTTP/1.1"
    );
}

#[tokio::test]
async fn apply_command_outcome_updates_status_and_catalog() {
    let client = Client::builder().build().expect("client should build");
    let runtime_handle = Handle::current();
    let server_url = "http://127.0.0.1:9".to_string();
    let auth_token = "developer".to_string();
    let session_id = "s_test".to_string();

    let app = tokio::task::spawn_blocking(move || {
        let context = build_context(
            &runtime_handle,
            &client,
            &server_url,
            &auth_token,
            &session_id,
        );
        let mut app = ChatApp::new("s_test", "http://127.0.0.1:8080", false, &[], &[], vec![]);
        apply_command_outcome(
            &context,
            &mut app,
            crate::repl_commands::ReplCommandOutcome {
                notices: vec![
                    crate::repl_commands::ReplCommandNotice::Help(vec![command_candidate(
                        "/help",
                        "/help",
                        "Show help",
                    )]),
                    crate::repl_commands::ReplCommandNotice::Status("working".to_string()),
                ],
                pending_permissions_update: crate::repl_commands::PendingPermissionsUpdate::None,
                should_quit: true,
            },
        )
        .expect("command outcome should apply");
        app
    })
    .await
    .expect("command outcome worker should join");

    assert!(app.should_quit());
    assert_eq!(app.command_catalog()[0].label, "/help");
    assert!(
        app.status_entries()
            .iter()
            .any(|status| status == "working")
    );
}

#[tokio::test]
async fn apply_command_outcome_refreshes_pending_permissions() {
    let (url, request_line_rx) = spawn_session_server(active_session(vec![PermissionRequest {
        request_id: "req_2".to_string(),
        summary: "read_text_file README.md".to_string(),
    }]))
    .await;
    let client = Client::builder().build().expect("client should build");
    let runtime_handle = Handle::current();
    let auth_token = "developer".to_string();
    let session_id = "s_test".to_string();
    let server_url = url.clone();

    let app = refreshed_permissions_app(runtime_handle, client, server_url, auth_token, session_id)
        .await
        .expect("command outcome worker should join");

    assert_eq!(app.pending_permissions()[0].request_id, "req_2");
    assert_eq!(
        request_line_rx
            .await
            .expect("request line should be captured"),
        "GET /api/v1/sessions/s_test HTTP/1.1"
    );
}

fn refreshed_permissions_app(
    runtime_handle: Handle,
    client: Client,
    server_url: String,
    auth_token: String,
    session_id: String,
) -> tokio::task::JoinHandle<ChatApp> {
    tokio::task::spawn_blocking(move || {
        let context = build_context(
            &runtime_handle,
            &client,
            &server_url,
            &auth_token,
            &session_id,
        );
        let mut app = ChatApp::new(
            "s_test",
            "http://127.0.0.1:8080",
            false,
            &[],
            &[PermissionRequest {
                request_id: "req_1".to_string(),
                summary: "old".to_string(),
            }],
            vec![],
        );
        apply_command_outcome(
            &context,
            &mut app,
            crate::repl_commands::ReplCommandOutcome {
                notices: vec![],
                pending_permissions_update: crate::repl_commands::PendingPermissionsUpdate::Refresh,
                should_quit: false,
            },
        )
        .expect("command outcome should apply");
        app
    })
}
