use super::support::*;
use acp_contracts::CreateSessionResponse;

#[tokio::test]
async fn browser_cookie_bootstrap_can_create_stream_and_prompt_a_session() -> Result<()> {
    let stack = TestStack::spawn(ServerConfig {
        session_cap: 8,
        acp_server: String::new(),
        startup_hints: false,
        frontend_dist: None,
    })
    .await?;
    let browser = build_browser_client()?;

    let app_document = load_browser_app_shell(&browser, &stack.backend_url).await?;
    // The shell must expose the CSRF bootstrap meta and the Leptos mount point.
    assert!(app_document.contains("name=\"acp-csrf-token\""));
    assert!(app_document.contains("id=\"app-root\""));

    let csrf_token = extract_meta_content(&app_document, "acp-csrf-token")?;
    let created: CreateSessionResponse =
        create_browser_session(&browser, &stack.backend_url, &csrf_token).await?;

    let session_id = created.session.id.clone();
    let mut events = open_cookie_events(&browser, &stack.backend_url, &session_id).await?;
    assert_snapshot_for_session(expect_next_event(&mut events).await?, &session_id);

    submit_browser_prompt(
        &browser,
        &stack.backend_url,
        &session_id,
        &csrf_token,
        "hello through the browser shell",
    )
    .await?;

    assert_user_message(
        expect_next_event(&mut events).await?,
        "hello through the browser shell",
    );
    assert_assistant_message(expect_next_event(&mut events).await?);

    Ok(())
}

fn assert_snapshot_for_session(event: StreamEvent, session_id: &str) {
    match event.payload {
        StreamEventPayload::SessionSnapshot { session } => {
            assert_eq!(session.id, session_id);
        }
        payload => panic!("expected session snapshot event, got {payload:?}"),
    }
}

fn assert_user_message(event: StreamEvent, expected_text: &str) {
    match event.payload {
        StreamEventPayload::ConversationMessage { message } => {
            assert!(matches!(message.role, MessageRole::User));
            assert_eq!(message.text, expected_text);
        }
        payload => panic!("expected user message event, got {payload:?}"),
    }
}

fn assert_assistant_message(event: StreamEvent) {
    match event.payload {
        StreamEventPayload::ConversationMessage { message } => {
            assert!(matches!(message.role, MessageRole::Assistant));
            assert!(message.text.starts_with("mock assistant:"));
        }
        payload => panic!("expected assistant message event, got {payload:?}"),
    }
}
