use super::support::*;
use acp_contracts::CreateSessionResponse;

#[tokio::test]
async fn browser_cookie_bootstrap_can_create_stream_and_prompt_a_session() -> Result<()> {
    let stack = spawn_browser_test_stack().await?;
    let browser = build_browser_client()?;
    let (csrf_token, session_id, mut events) =
        bootstrap_browser_session(&browser, &stack.backend_url).await?;

    submit_and_assert_browser_prompt(
        &browser,
        &stack.backend_url,
        &session_id,
        &csrf_token,
        &mut events,
        "hello through the browser shell",
    )
    .await?;

    Ok(())
}

async fn spawn_browser_test_stack() -> Result<TestStack> {
    TestStack::spawn(ServerConfig {
        session_cap: 8,
        acp_server: String::new(),
        startup_hints: false,
        frontend_dist: None,
    })
    .await
}

async fn bootstrap_browser_session(
    browser: &Client,
    backend_url: &str,
) -> Result<(String, String, SseStream)> {
    let app_document = load_browser_app_shell(browser, backend_url).await?;
    assert_browser_shell(&app_document);

    let csrf_token = extract_meta_content(&app_document, "acp-csrf-token")?;
    let created: CreateSessionResponse =
        create_browser_session(browser, backend_url, &csrf_token).await?;
    let session_id = created.session.id.clone();
    let mut events = open_cookie_events(browser, backend_url, &session_id).await?;
    assert_snapshot_for_session(expect_next_event(&mut events).await?, &session_id);
    Ok((csrf_token, session_id, events))
}

fn assert_browser_shell(app_document: &str) {
    assert!(app_document.contains("name=\"acp-csrf-token\""));
    assert!(app_document.contains("id=\"app-root\""));
}

async fn submit_and_assert_browser_prompt(
    browser: &Client,
    backend_url: &str,
    session_id: &str,
    csrf_token: &str,
    events: &mut SseStream,
    prompt: &str,
) -> Result<()> {
    submit_browser_prompt(browser, backend_url, session_id, csrf_token, prompt).await?;
    assert_user_message(expect_next_event(events).await?, prompt);
    assert_assistant_message(expect_next_event(events).await?);
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
