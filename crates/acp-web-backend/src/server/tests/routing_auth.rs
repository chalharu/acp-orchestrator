use super::*;

#[test]
fn default_server_config_points_to_the_local_acp_server() {
    let config = ServerConfig::default();

    assert_eq!(config.session_cap, 8);
    assert_eq!(config.acp_server, "127.0.0.1:8090");
    assert!(!config.startup_hints);
    assert_eq!(config.state_dir, std::path::PathBuf::from(".acp-state"));
}

#[test]
fn app_errors_map_to_the_expected_status_codes() {
    let cases = [
        (
            AppError::Unauthorized("auth".to_string()),
            StatusCode::UNAUTHORIZED,
            "auth",
        ),
        (
            AppError::Forbidden("forbidden".to_string()),
            StatusCode::FORBIDDEN,
            "forbidden",
        ),
        (
            AppError::NotFound("missing".to_string()),
            StatusCode::NOT_FOUND,
            "missing",
        ),
        (
            AppError::BadRequest("bad".to_string()),
            StatusCode::BAD_REQUEST,
            "bad",
        ),
        (
            AppError::Conflict("conflict".to_string()),
            StatusCode::CONFLICT,
            "conflict",
        ),
        (
            AppError::TooManyRequests("too many".to_string()),
            StatusCode::TOO_MANY_REQUESTS,
            "too many",
        ),
        (
            AppError::Internal("internal".to_string()),
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
        ),
    ];

    for (error, expected_status, expected_message) in cases {
        assert_eq!(error.status_code(), expected_status);
        assert_eq!(error.message(), expected_message);
    }
}

#[tokio::test]
async fn internal_errors_are_sanitized_in_http_responses() {
    let response = AppError::Internal("sensitive internal detail".to_string()).into_response();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body should be readable");
    let payload: crate::contract_health::ErrorResponse =
        serde_json::from_slice(&body).expect("error payload should decode");

    assert_eq!(payload.error, "internal server error");
}

#[test]
fn auth_errors_become_unauthorized_responses() {
    let missing: AppError = AuthError::MissingAuthentication.into();
    let invalid: AppError = AuthError::InvalidAuthentication.into();

    assert!(matches!(
        missing,
        AppError::Unauthorized(message) if message == "missing authentication"
    ));
    assert!(matches!(
        invalid,
        AppError::Unauthorized(message) if message == "invalid authentication"
    ));
}

#[test]
fn csrf_errors_become_forbidden_responses() {
    let missing: AppError = AuthError::MissingCsrfToken.into();
    let invalid: AppError = AuthError::InvalidCsrfToken.into();

    assert!(matches!(
        missing,
        AppError::Forbidden(message) if message == "missing csrf token"
    ));
    assert!(matches!(
        invalid,
        AppError::Forbidden(message) if message == "invalid csrf token"
    ));
}

#[tokio::test]
async fn app_entrypoint_bootstraps_browser_cookies_and_chat_shell_markup() {
    let response = app_entrypoint(HeaderMap::new()).await;
    let set_cookies = response
        .headers()
        .get_all(SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .map(str::to_string)
        .collect::<Vec<_>>();

    assert_eq!(set_cookies.len(), 2);
    assert!(
        set_cookies
            .iter()
            .any(|cookie| cookie.starts_with("acp_session=") && cookie.contains("HttpOnly"))
    );
    assert!(
        set_cookies
            .iter()
            .any(|cookie| cookie.starts_with("acp_csrf=") && !cookie.contains("HttpOnly"))
    );

    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("entrypoint body should be readable");
    let body = String::from_utf8(body.to_vec()).expect("entrypoint body should be UTF-8");

    // Shell document includes the CSRF bootstrap meta tag and the WASM loader.
    assert!(body.contains("name=\"acp-csrf-token\""));
    assert!(body.contains("wasm-init.js"));
    // The mount point element is present so the Leptos CSR app can attach.
    assert!(body.contains("id=\"app-root\""));
    // No hand-authored application JS is referenced.
    assert!(!body.contains("app.js"));
}

#[tokio::test]
async fn app_entrypoint_replaces_invalid_cookie_values_before_rendering() {
    let mut headers = HeaderMap::new();
    headers.insert(
        COOKIE,
        HeaderValue::from_static(r#"acp_session=not-a-uuid; acp_csrf="><bad"#),
    );

    let response = app_entrypoint(headers).await;
    let set_cookies = response
        .headers()
        .get_all(SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .map(str::to_string)
        .collect::<Vec<_>>();

    assert_eq!(set_cookies.len(), 2);

    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("entrypoint body should be readable");
    let body = String::from_utf8(body.to_vec()).expect("entrypoint body should be UTF-8");

    assert!(!body.contains(r#""><bad"#));
}

#[tokio::test]
async fn app_session_entrypoint_reuses_the_app_shell() {
    let response = app_session_entrypoint(Path("session-id".to_string()), HeaderMap::new()).await;
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("entrypoint body should be readable");
    let body = String::from_utf8(body.to_vec()).expect("entrypoint body should be UTF-8");

    // Both route variants serve the same Leptos CSR shell.
    assert!(body.contains("id=\"app-root\""));
    assert!(body.contains("wasm-init.js"));
}

#[tokio::test]
async fn app_sign_in_entrypoint_reuses_the_app_shell() {
    let response = app_sign_in_entrypoint(HeaderMap::new()).await;
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("entrypoint body should be readable");
    let body = String::from_utf8(body.to_vec()).expect("entrypoint body should be UTF-8");

    assert!(body.contains("id=\"app-root\""));
    assert!(body.contains("wasm-init.js"));
}

#[tokio::test]
async fn app_register_entrypoint_reuses_the_app_shell() {
    let response = app_register_entrypoint(HeaderMap::new()).await;
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("entrypoint body should be readable");
    let body = String::from_utf8(body.to_vec()).expect("entrypoint body should be UTF-8");

    assert!(body.contains("id=\"app-root\""));
    assert!(body.contains("wasm-init.js"));
}

#[tokio::test]
async fn app_accounts_entrypoint_reuses_the_app_shell() {
    let response = app_accounts_entrypoint(HeaderMap::new()).await;
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("entrypoint body should be readable");
    let body = String::from_utf8(body.to_vec()).expect("entrypoint body should be UTF-8");

    assert!(body.contains("id=\"app-root\""));
    assert!(body.contains("wasm-init.js"));
}

#[tokio::test]
async fn app_workspaces_entrypoint_reuses_the_app_shell() {
    let response = app_workspaces_entrypoint(HeaderMap::new()).await;
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("entrypoint body should be readable");
    let body = String::from_utf8(body.to_vec()).expect("entrypoint body should be UTF-8");

    assert!(body.contains("id=\"app-root\""));
    assert!(body.contains("wasm-init.js"));
}

#[tokio::test]
async fn bootstrap_registration_changes_auth_status_for_the_browser_session() {
    let state = AppState::with_workspace_repository(
        Arc::new(SessionStore::new(4)),
        new_ephemeral_workspace_repository(),
        Arc::new(StaticReplyProvider {
            reply: "test reply".to_string(),
        }),
    );
    let shell = app_entrypoint(HeaderMap::new()).await;
    let headers = browser_cookie_headers(&shell);
    let body = to_bytes(shell.into_body(), usize::MAX)
        .await
        .expect("entrypoint body should be readable");
    let body = String::from_utf8(body.to_vec()).expect("entrypoint body should be utf-8");
    let csrf_token = extract_meta_content(&body, "acp-csrf-token");

    let initial = auth_status(State(state.clone()), headers.clone())
        .await
        .expect("auth status should load");
    assert_eq!(
        initial.0,
        AuthStatusResponse {
            bootstrap_required: true,
            account: None,
        }
    );

    let mut write_headers = headers.clone();
    write_headers.insert("x-csrf-token", HeaderValue::from_str(&csrf_token).unwrap());
    let principal =
        authorize_request(&write_headers, true).expect("browser headers should authorize");
    let registered = bootstrap_register(
        State(state.clone()),
        Extension(principal),
        Json(BootstrapRegistrationRequest {
            username: "admin".to_string(),
            password: "password123".to_string(),
        }),
    )
    .await
    .expect("bootstrap registration should succeed");
    assert_eq!(registered.0, StatusCode::CREATED);
    assert_eq!(registered.1.0.account.username, "admin");

    let after = auth_status(State(state), headers)
        .await
        .expect("auth status should load after registration");
    assert!(after.0.account.is_some());
    assert!(!after.0.bootstrap_required);
}

#[tokio::test]
async fn bootstrap_registration_rejects_non_browser_principals() {
    let error = bootstrap_register(
        State(auth_test_state()),
        bearer_principal("developer"),
        Json(BootstrapRegistrationRequest {
            username: "admin".to_string(),
            password: "password123".to_string(),
        }),
    )
    .await
    .expect_err("bootstrap registration should reject bearer principals");

    assert!(matches!(
        error,
        AppError::Forbidden(message)
            if message == "bootstrap registration requires a browser session"
    ));
}

#[tokio::test]
async fn sign_in_rebinds_existing_accounts_to_a_new_browser_session() {
    let state = auth_test_state();
    let first_browser = BrowserAuthContext::spawn().await;
    bootstrap_admin_account(&state, &first_browser).await;
    let second_browser = BrowserAuthContext::spawn().await;

    let before = auth_status(State(state.clone()), second_browser.headers.clone())
        .await
        .expect("auth status should load before sign-in");
    assert!(before.0.account.is_none());
    assert!(!before.0.bootstrap_required);

    let signed_in = sign_in(
        State(state.clone()),
        Extension(second_browser.principal.clone()),
        Json(SignInRequest {
            username: "admin".to_string(),
            password: "password123".to_string(),
        }),
    )
    .await
    .expect("sign-in should succeed");
    assert_eq!(signed_in.0.account.username, "admin");

    let after = auth_status(State(state), second_browser.headers)
        .await
        .expect("auth status should load after sign-in");
    assert_eq!(after.0.account, Some(signed_in.0.account));
}

#[tokio::test]
async fn sign_in_rejects_non_browser_principals() {
    let error = sign_in(
        State(auth_test_state()),
        bearer_principal("developer"),
        Json(SignInRequest {
            username: "admin".to_string(),
            password: "password123".to_string(),
        }),
    )
    .await
    .expect_err("password sign-in should reject bearer principals");

    assert!(matches!(
        error,
        AppError::Forbidden(message)
            if message == "password sign-in requires a browser session"
    ));
}

#[tokio::test]
async fn sign_in_clears_live_sessions_before_rebinding_a_browser_session() {
    let state = auth_test_state();
    let browser = BrowserAuthContext::spawn().await;
    bootstrap_admin_account(&state, &browser).await;
    let _workspace = create_owned_workspace_for_principal(
        &state,
        Extension(browser.principal.clone()),
        "Browser Workspace",
    )
    .await;
    let created = create_session(
        State(state.clone()),
        Extension(browser.principal.clone()),
        axum::body::Bytes::new(),
    )
    .await
    .expect("session creation should succeed")
    .1
    .0
    .session;
    state
        .workspace_repository
        .create_local_account("member", "password123", false)
        .await
        .expect("member creation should succeed");

    let _signed_in = sign_in(
        State(state.clone()),
        Extension(browser.principal.clone()),
        Json(SignInRequest {
            username: "member".to_string(),
            password: "password123".to_string(),
        }),
    )
    .await
    .expect("sign-in should succeed");

    let snapshot_error = state
        .store
        .session_snapshot(&browser.principal.id, &created.id)
        .await
        .expect_err("rebound browser sessions should lose prior live chats");
    assert_eq!(snapshot_error, SessionStoreError::NotFound);
}

#[tokio::test]
async fn list_sessions_returns_owned_sessions_for_the_authenticated_principal() {
    let state = auth_test_state();
    let session = state
        .store
        .create_session("alice", "w_test")
        .await
        .expect("session creation should succeed");
    state
        .store
        .create_session("bob", "w_test")
        .await
        .expect("other session creation should succeed");

    let response = list_sessions(State(state), bearer_principal("alice"))
        .await
        .expect("listing sessions should succeed");

    assert_eq!(response.0.sessions.len(), 1);
    assert_eq!(response.0.sessions[0].id, session.id);
    assert_eq!(response.0.sessions[0].title, session.title);
    assert_eq!(response.0.sessions[0].status, session.status);
}

#[tokio::test]
async fn get_session_returns_the_requested_owned_session() {
    let state = auth_test_state();
    let session = state
        .store
        .create_session("alice", "w_test")
        .await
        .expect("session creation should succeed");

    let response = get_session(
        State(state),
        Path(session.id.clone()),
        bearer_principal("alice"),
    )
    .await
    .expect("reading the session should succeed");

    assert_eq!(response.0.session, session);
}

#[tokio::test]
async fn sign_out_clears_browser_authentication_and_cookies() {
    let state = auth_test_state();
    let browser = BrowserAuthContext::spawn().await;
    bootstrap_admin_account(&state, &browser).await;

    let response = sign_out(State(state.clone()), Extension(browser.principal.clone()))
        .await
        .expect("sign-out should succeed");
    let set_cookies = response
        .headers()
        .get_all(SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .map(str::to_string)
        .collect::<Vec<_>>();
    let after = auth_status(State(state), browser.headers)
        .await
        .expect("auth status should load after sign-out");

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(set_cookies.iter().any(|cookie| {
        cookie.starts_with("acp_session=deleted")
            && cookie.contains("Max-Age=0")
            && cookie.contains("HttpOnly")
    }));
    assert!(set_cookies.iter().any(|cookie| {
        cookie.starts_with("acp_csrf=deleted")
            && cookie.contains("Max-Age=0")
            && !cookie.contains("HttpOnly")
    }));
    assert!(!after.0.bootstrap_required);
    assert!(after.0.account.is_none());
}

#[tokio::test]
async fn sign_out_rejects_non_browser_principals() {
    let error = sign_out(State(auth_test_state()), bearer_principal("developer"))
        .await
        .expect_err("sign-out should reject bearer principals");

    assert!(matches!(
        error,
        AppError::Forbidden(message) if message == "sign-out requires a browser session"
    ));
}

#[tokio::test]
async fn admin_account_handlers_list_and_update_accounts() {
    let state = auth_test_state();
    let admin_browser = BrowserAuthContext::spawn().await;
    bootstrap_admin_account(&state, &admin_browser).await;
    let member = create_member_account(&state, &admin_browser, "member", "password123").await;

    let listed = list_accounts(
        State(state.clone()),
        Extension(admin_browser.principal.clone()),
    )
    .await
    .expect("listing accounts should succeed");
    assert_eq!(listed.0.accounts.len(), 2);
    assert!(
        listed
            .0
            .accounts
            .iter()
            .any(|account| account.user_id == member.user_id)
    );

    let updated = update_account(
        State(state.clone()),
        Path(member.user_id.clone()),
        Extension(admin_browser.principal.clone()),
        Json(UpdateAccountRequest {
            password: Some("password456".to_string()),
            is_admin: Some(true),
        }),
    )
    .await
    .expect("updating the member account should succeed");
    assert!(updated.0.account.is_admin);
}

#[tokio::test]
async fn admin_account_deletions_forget_live_sessions() {
    let (state, forgotten_sessions) = tracking_auth_test_state();
    let admin_browser = BrowserAuthContext::spawn().await;
    bootstrap_admin_account(&state, &admin_browser).await;
    let member = create_member_account(&state, &admin_browser, "member", "password123").await;
    let member_browser = BrowserAuthContext::spawn().await;
    sign_in_browser_account(&state, &member_browser, "member", "password123").await;
    let live_session = state
        .store
        .create_session(&member_browser.principal.id, "w_test")
        .await
        .expect("member live session should create");

    let deleted = delete_account(
        State(state.clone()),
        Path(member.user_id),
        Extension(admin_browser.principal),
    )
    .await
    .expect("deleting the member account should succeed");
    assert!(deleted.0.deleted);
    assert_eq!(
        state
            .store
            .session_snapshot(&member_browser.principal.id, &live_session.id)
            .await,
        Err(SessionStoreError::NotFound)
    );
    assert_eq!(
        forgotten_sessions
            .lock()
            .expect("tracking should not poison")
            .as_slice(),
        &[live_session.id]
    );
}

#[tokio::test]
async fn account_handlers_require_admin_access() {
    let state = auth_test_state();
    let admin_browser = BrowserAuthContext::spawn().await;
    bootstrap_admin_account(&state, &admin_browser).await;
    create_member_account(&state, &admin_browser, "member", "password123").await;
    let member_browser = BrowserAuthContext::spawn().await;
    sign_in_browser_account(&state, &member_browser, "member", "password123").await;

    let error = list_accounts(State(state), Extension(member_browser.principal))
        .await
        .expect_err("non-admin users should not list accounts");
    assert!(matches!(
        error,
        AppError::Forbidden(message) if message == "admin access required"
    ));
}
