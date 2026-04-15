use super::*;

#[tokio::test]
async fn submit_current_input_handles_blank_prompts_and_submits_messages() {
    let (url, request_line_rx, request_body_rx) = spawn_prompt_server().await;
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
        let mut blank_app = base_app();
        submit_current_input(&context, &mut blank_app).expect("blank input should be ignored");

        let mut app = base_app();
        for value in "  hello  ".chars() {
            app.insert_char(value);
        }
        submit_current_input(&context, &mut app).expect("prompt submission should succeed");
        app
    })
    .await
    .expect("prompt worker should join");

    assert_eq!(app.input(), "");
    assert_eq!(
        request_line_rx
            .await
            .expect("request line should be captured"),
        "POST /api/v1/sessions/s_test/messages HTTP/1.1"
    );
    assert_eq!(
        request_body_rx
            .await
            .expect("request body should be captured"),
        "{\"text\":\"hello\"}"
    );
}

#[test]
fn submit_current_input_handles_quit_commands() {
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
    for value in "/quit".chars() {
        app.insert_char(value);
    }

    submit_current_input(&context, &mut app).expect("quit should stay in-process");

    assert!(app.should_quit());
    assert_eq!(app.input(), "");
}

#[tokio::test]
async fn handle_paste_submits_complete_lines_and_keeps_partial_input() {
    let (url, request_line_rx, request_body_rx) = spawn_prompt_server().await;
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
        handle_paste(&context, &mut app, "hello\nworld").expect("pasted input should be processed");
        app
    })
    .await
    .expect("paste worker should join");

    assert_eq!(app.input(), "world");
    assert_eq!(
        request_line_rx
            .await
            .expect("request line should be captured"),
        "POST /api/v1/sessions/s_test/messages HTTP/1.1"
    );
    assert_eq!(
        request_body_rx
            .await
            .expect("request body should be captured"),
        "{\"text\":\"hello\"}"
    );
}

#[tokio::test]
async fn submit_current_input_routes_slash_commands_and_prompt_failures() {
    let (url, _request_line_rx) = spawn_completion_server(SlashCompletionsResponse {
        candidates: vec![command_candidate("/help", "/help", "Show help")],
    })
    .await;
    let client = Client::builder().build().expect("client should build");
    let runtime_handle = Handle::current();
    let auth_token = "developer".to_string();
    let session_id = "s_test".to_string();
    let server_url = url.clone();

    let help_app = tokio::task::spawn_blocking(move || {
        let context = build_context(
            &runtime_handle,
            &client,
            &server_url,
            &auth_token,
            &session_id,
        );
        let mut app = base_app();
        for value in "/help".chars() {
            app.insert_char(value);
        }
        submit_current_input(&context, &mut app).expect("slash commands should stay in-process");
        app
    })
    .await
    .expect("help worker should join");

    assert!(help_app.input().is_empty());
    assert_eq!(help_app.command_catalog()[0].label, "/help");
    assert!(
        help_app
            .status_entries()
            .iter()
            .any(|status| status == "available slash commands refreshed")
    );

    let client = Client::builder().build().expect("client should build");
    let runtime_handle = Handle::current();
    let auth_token = "developer".to_string();
    let session_id = "s_test".to_string();
    let server_url = "http://127.0.0.1:9".to_string();

    let failed_prompt_app = tokio::task::spawn_blocking(move || {
        let context = build_context(
            &runtime_handle,
            &client,
            &server_url,
            &auth_token,
            &session_id,
        );
        let mut app = base_app();
        for value in "hello".chars() {
            app.insert_char(value);
        }
        submit_current_input(&context, &mut app).expect("prompt failures should stay in-process");
        app
    })
    .await
    .expect("failed prompt worker should join");

    assert!(
        failed_prompt_app
            .status_entries()
            .iter()
            .any(|status| status == "submit prompt request failed")
    );
}

#[tokio::test]
async fn handle_key_enter_routes_plain_and_slash_input_through_submit_current_input() {
    let (prompt_url, prompt_request_line_rx, _prompt_body_rx) = spawn_prompt_server().await;
    let client = Client::builder().build().expect("client should build");
    let runtime_handle = Handle::current();
    let auth_token = "developer".to_string();
    let session_id = "s_test".to_string();
    let server_url = prompt_url.clone();

    let prompt_app = tokio::task::spawn_blocking(move || {
        let context = build_context(
            &runtime_handle,
            &client,
            &server_url,
            &auth_token,
            &session_id,
        );
        let mut app = base_app();
        for value in "hello".chars() {
            app.insert_char(value);
        }
        handle_key(
            &context,
            &mut app,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            3,
            40,
        )
        .expect("enter should submit prompt input");
        app
    })
    .await
    .expect("plain enter worker should join");

    assert!(prompt_app.input().is_empty());
    assert_eq!(
        prompt_request_line_rx
            .await
            .expect("prompt request line should be captured"),
        "POST /api/v1/sessions/s_test/messages HTTP/1.1"
    );

    let (help_url, _request_line_rx) = spawn_completion_server(SlashCompletionsResponse {
        candidates: vec![command_candidate("/help", "/help", "Show help")],
    })
    .await;
    let client = Client::builder().build().expect("client should build");
    let runtime_handle = Handle::current();
    let auth_token = "developer".to_string();
    let session_id = "s_test".to_string();
    let server_url = help_url.clone();

    let help_app = tokio::task::spawn_blocking(move || {
        let context = build_context(
            &runtime_handle,
            &client,
            &server_url,
            &auth_token,
            &session_id,
        );
        let mut app = base_app();
        for value in "/help".chars() {
            app.insert_char(value);
        }
        handle_key(
            &context,
            &mut app,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            3,
            40,
        )
        .expect("enter should execute slash commands");
        app
    })
    .await
    .expect("slash enter worker should join");

    assert!(help_app.input().is_empty());
    assert_eq!(help_app.command_catalog()[0].label, "/help");
}
