use super::support::*;
use acp_contracts::{CreateSessionResponse, PromptRequest};

#[tokio::test]
async fn browser_cookie_bootstrap_can_create_stream_and_prompt_a_session() -> Result<()> {
    let stack = TestStack::spawn(ServerConfig {
        session_cap: 8,
        acp_server: String::new(),
        startup_hints: false,
    })
    .await?;
    let browser = build_browser_client()?;

    let app_document = browser
        .get(format!("{}/app/", stack.backend_url))
        .send()
        .await
        .context("loading the browser app shell")?
        .error_for_status()
        .context("browser app shell returned an error")?
        .text()
        .await
        .context("reading the browser app shell")?;
    assert!(app_document.contains("ACP Web MVP slice 1"));

    let csrf_token = extract_meta_content(&app_document, "acp-csrf-token")?;
    let created: CreateSessionResponse = browser
        .post(format!("{}/api/v1/sessions", stack.backend_url))
        .header("x-csrf-token", &csrf_token)
        .send()
        .await
        .context("creating a cookie-authenticated browser session")?
        .error_for_status()
        .context("cookie-authenticated browser session creation returned an error")?
        .json()
        .await
        .context("decoding the created browser session")?;

    let session_id = created.session.id.clone();
    let mut events = open_cookie_events(&browser, &stack.backend_url, &session_id).await?;
    let snapshot = expect_next_event(&mut events).await?;
    assert!(matches!(
        snapshot.payload,
        StreamEventPayload::SessionSnapshot { ref session } if session.id == session_id
    ));

    browser
        .post(format!(
            "{}/api/v1/sessions/{session_id}/messages",
            stack.backend_url
        ))
        .header("x-csrf-token", &csrf_token)
        .json(&PromptRequest {
            text: "hello through the browser shell".to_string(),
        })
        .send()
        .await
        .context("submitting a browser-authenticated prompt")?
        .error_for_status()
        .context("browser-authenticated prompt submission returned an error")?;

    let user_message = expect_next_event(&mut events).await?;
    assert!(matches!(
        user_message.payload,
        StreamEventPayload::ConversationMessage { ref message }
            if matches!(message.role, MessageRole::User)
                && message.text == "hello through the browser shell"
    ));

    let assistant_message = expect_next_event(&mut events).await?;
    assert!(matches!(
        assistant_message.payload,
        StreamEventPayload::ConversationMessage { ref message }
            if matches!(message.role, MessageRole::Assistant)
                && message.text.starts_with("mock assistant:")
    ));

    Ok(())
}
