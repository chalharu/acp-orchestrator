use std::time::Duration;

use super::support::*;
use acp_contracts::{AuthSessionResponse, CreateSessionResponse, SignUpRequest};
use acp_mock::MANUAL_PERMISSION_TRIGGER;
use futures_util::StreamExt;

const BROWSER_TEST_USER_NAME: &str = "browser-test";
const BROWSER_TEST_PASSWORD: &str = "browser-test-password";
const BROWSER_SWITCHED_USER_NAME: &str = "browser-test-switched";
const BROWSER_SWITCHED_PASSWORD: &str = "browser-test-switched-password";

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

#[tokio::test]
async fn browser_cookie_bootstrap_can_resolve_pending_permissions() -> Result<()> {
    let stack = spawn_browser_test_stack().await?;
    let browser = build_browser_client()?;
    let (csrf_token, session_id, mut events) =
        bootstrap_browser_session(&browser, &stack.backend_url).await?;

    let request = submit_permission_prompt_and_wait(
        &browser,
        &stack.backend_url,
        &session_id,
        &csrf_token,
        &mut events,
    )
    .await?;
    let resolution = resolve_browser_permission(
        &browser,
        &stack.backend_url,
        &session_id,
        &request.request_id,
        &csrf_token,
        PermissionDecision::Approve,
    )
    .await?;

    assert_eq!(resolution.request_id, request.request_id);
    assert_snapshot_without_pending_permissions(expect_next_event(&mut events).await?);
    assert_assistant_message(expect_next_event(&mut events).await?);
    Ok(())
}

#[tokio::test]
async fn browser_cookie_bootstrap_can_cancel_pending_permission_turns() -> Result<()> {
    let stack = spawn_browser_test_stack().await?;
    let browser = build_browser_client()?;
    let (csrf_token, session_id, mut events) =
        bootstrap_browser_session(&browser, &stack.backend_url).await?;

    let _ = submit_permission_prompt_and_wait(
        &browser,
        &stack.backend_url,
        &session_id,
        &csrf_token,
        &mut events,
    )
    .await?;

    let cancelled =
        cancel_browser_turn(&browser, &stack.backend_url, &session_id, &csrf_token).await?;
    assert!(cancelled.cancelled);

    assert_snapshot_without_pending_permissions(expect_next_event(&mut events).await?);
    assert_cancelled_status(expect_next_event(&mut events).await?);
    Ok(())
}

#[tokio::test]
async fn browser_sign_out_closes_open_event_streams() -> Result<()> {
    let stack = spawn_browser_test_stack().await?;
    let browser = build_browser_client()?;
    let (csrf_token, _session_id, mut events) =
        bootstrap_browser_session(&browser, &stack.backend_url).await?;

    let signed_out = sign_out_browser_session(&browser, &stack.backend_url, &csrf_token).await?;
    assert!(!signed_out.authenticated);
    assert!(!signed_out.is_admin);
    assert!(!signed_out.bootstrap_registration_open);
    assert_eq!(signed_out.user_name, None);

    assert_browser_stream_closes(&mut events).await?;
    Ok(())
}

#[tokio::test]
async fn browser_re_sign_in_closes_stale_open_event_streams() -> Result<()> {
    let stack = spawn_browser_test_stack().await?;
    let browser = build_browser_client()?;
    let (csrf_token, _session_id, mut events) =
        bootstrap_browser_session(&browser, &stack.backend_url).await?;
    register_additional_browser_account(
        &browser,
        &stack.backend_url,
        BROWSER_TEST_USER_NAME,
        BROWSER_SWITCHED_USER_NAME,
        BROWSER_SWITCHED_PASSWORD,
    )
    .await?;

    assert_browser_sign_in(
        sign_in_browser_session(
            &browser,
            &stack.backend_url,
            &csrf_token,
            BROWSER_SWITCHED_USER_NAME,
            BROWSER_SWITCHED_PASSWORD,
        )
        .await?,
        BROWSER_SWITCHED_USER_NAME,
        false,
    );

    assert_browser_stream_closes(&mut events).await?;
    Ok(())
}

#[tokio::test]
async fn browser_cookie_registration_requires_bootstrap_or_admin_access() -> Result<()> {
    let stack = spawn_browser_test_stack().await?;
    let admin_browser = build_browser_client()?;
    let app_document = load_browser_app_shell(&admin_browser, &stack.backend_url).await?;
    let csrf_token = extract_meta_content(&app_document, "acp-csrf-token")?;
    assert_browser_sign_in(
        register_browser_account(
            &admin_browser,
            &stack.backend_url,
            &csrf_token,
            BROWSER_TEST_USER_NAME,
            BROWSER_TEST_PASSWORD,
        )
        .await?,
        BROWSER_TEST_USER_NAME,
        true,
    );

    let unauthenticated_browser = build_browser_client()?;
    let app_document = load_browser_app_shell(&unauthenticated_browser, &stack.backend_url).await?;
    let csrf_token = extract_meta_content(&app_document, "acp-csrf-token")?;
    let response = unauthenticated_browser
        .post(format!("{}/api/v1/auth/register", stack.backend_url))
        .header("x-csrf-token", &csrf_token)
        .json(&SignUpRequest {
            user_name: "blocked".to_string(),
            password: BROWSER_TEST_PASSWORD.to_string(),
        })
        .send()
        .await
        .context("submitting an unauthenticated post-bootstrap registration")?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    register_additional_browser_account(
        &admin_browser,
        &stack.backend_url,
        BROWSER_TEST_USER_NAME,
        BROWSER_SWITCHED_USER_NAME,
        BROWSER_SWITCHED_PASSWORD,
    )
    .await?;
    Ok(())
}

async fn spawn_browser_test_stack() -> Result<TestStack> {
    TestStack::spawn(ServerConfig {
        session_cap: 8,
        acp_server: String::new(),
        startup_hints: false,
        state_dir: test_state_dir(),
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
    assert_browser_sign_in(
        register_browser_account(
            browser,
            backend_url,
            &csrf_token,
            BROWSER_TEST_USER_NAME,
            BROWSER_TEST_PASSWORD,
        )
        .await?,
        BROWSER_TEST_USER_NAME,
        true,
    );
    let created: CreateSessionResponse =
        create_browser_session(browser, backend_url, &csrf_token).await?;
    let session_id = created.session.id.clone();
    let mut events = open_cookie_events(browser, backend_url, &session_id).await?;
    assert_snapshot_for_session(expect_next_event(&mut events).await?, &session_id);
    Ok((csrf_token, session_id, events))
}

async fn register_additional_browser_account(
    browser: &Client,
    backend_url: &str,
    current_admin_user_name: &str,
    user_name: &str,
    password: &str,
) -> Result<()> {
    let app_document = load_browser_app_shell(browser, backend_url).await?;
    let csrf_token = extract_meta_content(&app_document, "acp-csrf-token")?;
    assert_browser_sign_in(
        register_browser_account(browser, backend_url, &csrf_token, user_name, password).await?,
        current_admin_user_name,
        true,
    );
    Ok(())
}

fn assert_browser_shell(app_document: &str) {
    assert!(app_document.contains("name=\"acp-csrf-token\""));
    assert!(app_document.contains("id=\"app-root\""));
}

fn assert_browser_sign_in(
    response: AuthSessionResponse,
    expected_user_name: &str,
    expected_is_admin: bool,
) {
    assert!(response.authenticated);
    assert_eq!(response.is_admin, expected_is_admin);
    assert!(!response.bootstrap_registration_open);
    assert_eq!(response.user_name.as_deref(), Some(expected_user_name));
}

async fn assert_browser_stream_closes(events: &mut SseStream) -> Result<()> {
    let next = tokio::time::timeout(Duration::from_secs(2), events.next())
        .await
        .context("timed out waiting for the browser event stream to close")?;
    assert!(
        next.is_none(),
        "expected the browser event stream to close when authentication changes"
    );
    Ok(())
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

async fn submit_permission_prompt_and_wait(
    browser: &Client,
    backend_url: &str,
    session_id: &str,
    csrf_token: &str,
    events: &mut SseStream,
) -> Result<acp_contracts::PermissionRequest> {
    submit_browser_prompt(
        browser,
        backend_url,
        session_id,
        csrf_token,
        MANUAL_PERMISSION_TRIGGER,
    )
    .await?;
    assert_user_message(expect_next_event(events).await?, MANUAL_PERMISSION_TRIGGER);
    next_permission_request(events).await
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

async fn next_permission_request(
    events: &mut SseStream,
) -> Result<acp_contracts::PermissionRequest> {
    match expect_next_event(events).await?.payload {
        StreamEventPayload::PermissionRequested { request } => Ok(request),
        payload => panic!("expected permission request event, got {payload:?}"),
    }
}

fn assert_snapshot_without_pending_permissions(event: StreamEvent) {
    assert!(matches!(
        event.payload,
        StreamEventPayload::SessionSnapshot { session } if session.pending_permissions.is_empty()
    ));
}

fn assert_cancelled_status(event: StreamEvent) {
    assert!(matches!(
        event.payload,
        StreamEventPayload::Status { message } if message == "turn cancelled"
    ));
}
