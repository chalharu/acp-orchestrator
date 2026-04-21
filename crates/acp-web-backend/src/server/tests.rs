use super::*;
use crate::mock_client::{MockClientError, ReplyFuture, ReplyResult};
use crate::workspace_repository::WorkspaceRepository;
use crate::workspace_store::{
    SessionMetadataRecord, SqliteWorkspaceRepository, UserRecord, WorkspaceRecord,
};
use acp_app_support::{FrontendBundleAsset, build_http_client_for_url, frontend_bundle_file_name};
use acp_contracts::{
    AuthStatusResponse, BootstrapRegistrationRequest, SessionSnapshot, SessionStatus, SignInRequest,
};
use async_trait::async_trait;
use axum::{
    body::{Body, to_bytes},
    extract::Extension,
    http::{
        HeaderValue,
        header::{COOKIE, SET_COOKIE},
    },
    response::Response,
};
use std::sync::{Arc as StdArc, Mutex};
use tokio::time::timeout;
use tower::ServiceExt;

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
    let created = create_session(State(state.clone()), Extension(browser.principal.clone()))
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
        Json(acp_contracts::UpdateAccountRequest {
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
        .create_session(&member_browser.principal.id)
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

#[tokio::test]
async fn redirect_to_app_uses_the_canonical_trailing_slash_route() {
    let response = redirect_to_app().await.into_response();
    let location = response
        .headers()
        .get(axum::http::header::LOCATION)
        .expect("redirect responses should include a location header");

    assert_eq!(response.status(), StatusCode::PERMANENT_REDIRECT);
    assert_eq!(location.to_str().ok(), Some("/app/"));
}

#[tokio::test]
async fn redirect_to_register_uses_the_canonical_trailing_slash_route() {
    let response = redirect_to_register().await.into_response();
    let location = response
        .headers()
        .get(axum::http::header::LOCATION)
        .expect("redirect responses should include a location header");

    assert_eq!(response.status(), StatusCode::PERMANENT_REDIRECT);
    assert_eq!(location.to_str().ok(), Some("/app/register/"));
}

#[tokio::test]
async fn redirect_to_sign_in_uses_the_canonical_trailing_slash_route() {
    let response = redirect_to_sign_in().await.into_response();
    let location = response
        .headers()
        .get(axum::http::header::LOCATION)
        .expect("redirect responses should include a location header");

    assert_eq!(response.status(), StatusCode::PERMANENT_REDIRECT);
    assert_eq!(location.to_str().ok(), Some("/app/sign-in/"));
}

#[tokio::test]
async fn redirect_to_accounts_uses_the_canonical_trailing_slash_route() {
    let response = redirect_to_accounts().await.into_response();
    let location = response
        .headers()
        .get(axum::http::header::LOCATION)
        .expect("redirect responses should include a location header");

    assert_eq!(response.status(), StatusCode::PERMANENT_REDIRECT);
    assert_eq!(location.to_str().ok(), Some("/app/accounts/"));
}

#[tokio::test]
async fn app_shell_csp_permits_wasm_execution() {
    let response = app_entrypoint(HeaderMap::new()).await;
    let csp = response
        .headers()
        .get("content-security-policy")
        .expect("app shell should include a CSP header")
        .to_str()
        .expect("CSP header should be valid UTF-8");

    // WebAssembly execution requires 'wasm-unsafe-eval' in script-src.
    assert!(
        csp.contains("'wasm-unsafe-eval'"),
        "CSP script-src must include 'wasm-unsafe-eval' for WASM; got: {csp}",
    );
}

#[tokio::test]
async fn wasm_init_script_responds_with_javascript_content_type() {
    let response = wasm_init_script().await;
    let ct = response
        .headers()
        .get(CONTENT_TYPE)
        .expect("wasm-init.js response should include content-type")
        .to_str()
        .expect("content-type should be valid UTF-8");

    assert!(ct.starts_with("application/javascript"), "got: {ct}");
}

#[tokio::test]
async fn app_stylesheet_responds_with_css_content_type() {
    let response = app_stylesheet().await;
    let ct = response
        .headers()
        .get(CONTENT_TYPE)
        .expect("app.css response should include content-type")
        .to_str()
        .expect("content-type should be valid UTF-8")
        .to_string();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("app.css body should be readable");
    let body_text = String::from_utf8(body.to_vec()).expect("app.css should be valid UTF-8");

    assert!(ct.starts_with("text/css"), "got: {ct}");
    assert!(!body_text.is_empty());
    assert!(body_text.contains("Noto Sans JP"));
    assert!(body_text.contains(
        ".account-shell {\n  width: min(1160px, 100%);\n  height: 100%;\n  min-height: 0;\n  overflow-y: auto;"
    ));
    assert!(body_text.contains(".account-table-wrap {\n  overflow: auto;"));
}

#[tokio::test]
async fn app_font_asset_responds_with_font_content_type() {
    for font_name in [
        "noto-sans-jp-latin-400.woff2",
        "noto-sans-jp-japanese-400.woff2",
        "noto-sans-jp-latin-500.woff2",
        "noto-sans-jp-japanese-500.woff2",
        "noto-sans-jp-latin-700.woff2",
        "noto-sans-jp-japanese-700.woff2",
    ] {
        let response = app_font_asset(Path(font_name.to_string())).await;
        let ct = response
            .headers()
            .get(CONTENT_TYPE)
            .expect("font asset response should include content-type")
            .to_str()
            .expect("content-type should be valid UTF-8")
            .to_string();
        let cache_control = response
            .headers()
            .get(CACHE_CONTROL)
            .expect("font asset response should include cache-control")
            .to_str()
            .expect("cache-control should be valid UTF-8")
            .to_string();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("font asset body should be readable");

        assert!(ct.starts_with("font/woff2"), "{font_name}: got {ct}");
        assert_eq!(
            cache_control, "public, max-age=31536000, immutable",
            "{font_name}: cache-control mismatch"
        );
        assert!(!body.is_empty(), "{font_name}: body should not be empty");
    }
}

#[tokio::test]
async fn app_font_asset_returns_not_found_for_unknown_names() {
    let response = app_font_asset(Path("missing.ttf".to_string())).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn wasm_glue_js_returns_503_when_frontend_dist_is_not_configured() {
    let state = test_state(); // frontend_dist = None
    let response = wasm_glue_javascript(State(state)).await;

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn wasm_glue_js_returns_503_when_frontend_js_bundle_is_missing() {
    let response = wasm_glue_javascript(State(test_state_with_frontend_dist(
        write_temp_frontend_dist_with(false, true),
    )))
    .await;

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn wasm_glue_js_returns_503_when_frontend_js_bundle_cannot_be_read() {
    let response = wasm_glue_javascript(State(test_state_with_frontend_dist(
        write_temp_frontend_dist_with_unreadable_javascript(),
    )))
    .await;

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn wasm_glue_js_responds_with_javascript_content_type_when_dist_is_configured() {
    let response = wasm_glue_javascript(State(test_state_with_frontend_dist(
        write_temp_frontend_dist(),
    )))
    .await;

    let ct = response
        .headers()
        .get(CONTENT_TYPE)
        .expect("WASM glue JS response should include content-type")
        .to_str()
        .expect("content-type should be valid UTF-8");

    assert!(ct.starts_with("application/javascript"), "got: {ct}");
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn wasm_binary_returns_503_when_frontend_dist_is_not_configured() {
    let state = test_state(); // frontend_dist = None
    let response = wasm_binary(State(state)).await;

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn wasm_binary_returns_503_when_frontend_wasm_bundle_is_missing() {
    let response = wasm_binary(State(test_state_with_frontend_dist(
        write_temp_frontend_dist_with(true, false),
    )))
    .await;

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn wasm_binary_returns_503_when_frontend_wasm_bundle_cannot_be_read() {
    let response = wasm_binary(State(test_state_with_frontend_dist(
        write_temp_frontend_dist_with_unreadable_wasm(),
    )))
    .await;

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn wasm_binary_responds_with_wasm_content_type_when_dist_is_configured() {
    let response = wasm_binary(State(test_state_with_frontend_dist(
        write_temp_frontend_dist(),
    )))
    .await;

    let ct = response
        .headers()
        .get(CONTENT_TYPE)
        .expect("WASM binary response should include content-type")
        .to_str()
        .expect("content-type should be valid UTF-8");

    assert_eq!(ct, "application/wasm", "got: {ct}");
    assert_eq!(response.status(), StatusCode::OK);
}

/// Creates a temporary directory that looks like a minimal Trunk dist directory.
fn write_temp_frontend_dist() -> std::path::PathBuf {
    write_temp_frontend_dist_with(true, true)
}

fn write_temp_frontend_dist_with(
    include_javascript: bool,
    include_wasm: bool,
) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("acp-test-frontend-dist-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("temp dist dir should be creatable");
    if include_javascript {
        std::fs::write(
            dir.join(frontend_bundle_file_name(
                "test",
                FrontendBundleAsset::JavaScript,
            )),
            b"// stub js loader",
        )
        .expect("stub JS should be writable");
    }
    if include_wasm {
        std::fs::write(
            dir.join(frontend_bundle_file_name("test", FrontendBundleAsset::Wasm)),
            b"\x00asm\x01\x00\x00\x00", // minimal valid WASM header
        )
        .expect("stub WASM should be writable");
    }
    dir
}

fn write_temp_frontend_dist_with_unreadable_javascript() -> std::path::PathBuf {
    let dir = write_temp_frontend_dist_with(false, true);
    std::fs::create_dir(dir.join(frontend_bundle_file_name(
        "test",
        FrontendBundleAsset::JavaScript,
    )))
    .expect("stub unreadable JS directory should be creatable");
    dir
}

fn write_temp_frontend_dist_with_unreadable_wasm() -> std::path::PathBuf {
    let dir = write_temp_frontend_dist_with(true, false);
    std::fs::create_dir(dir.join(frontend_bundle_file_name("test", FrontendBundleAsset::Wasm)))
        .expect("stub unreadable WASM directory should be creatable");
    dir
}

fn test_state_with_frontend_dist(dist: std::path::PathBuf) -> AppState {
    AppState {
        store: Arc::new(SessionStore::new(1)),
        workspace_repository: new_ephemeral_workspace_repository(),
        reply_provider: Arc::new(StaticReplyProvider {
            reply: String::new(),
        }),
        startup_hints: false,
        frontend_dist: Some(Arc::new(dist)),
    }
}

#[tokio::test]
async fn serving_with_shutdown_handles_successful_connections() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("test listener should bind");
    let address = listener
        .local_addr()
        .expect("test listener should expose its address");
    let base_url = format!("https://{address}");
    let client = build_http_client_for_url(&base_url, Some(Duration::from_secs(1)))
        .expect("loopback clients should build");
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        serve_with_shutdown(listener, test_state(), async move {
            let _ = shutdown_rx.await;
        })
        .await
    });

    let response = client
        .get(format!("{base_url}/healthz"))
        .send()
        .await
        .expect("health requests should reach the server");
    response
        .error_for_status()
        .expect("health requests should succeed")
        .bytes()
        .await
        .expect("health responses should be readable");

    drop(client);
    tokio::task::yield_now().await;
    shutdown_tx
        .send(())
        .expect("shutdown signals should reach the server");

    timeout(Duration::from_secs(1), server)
        .await
        .expect("the server should stop promptly")
        .expect("the server task should join")
        .expect("serving should shut down cleanly");
}

#[tokio::test]
async fn aborted_connection_tasks_are_logged_without_panicking() {
    let mut connections = tokio::task::JoinSet::new();
    connections.spawn(async {
        panic!("boom");
    });

    let next = connections.join_next().await;
    log_connection_task_join_result(next);

    assert!(connections.is_empty());
}

#[test]
fn connection_results_are_logged_without_panicking() {
    log_connection_result(Ok::<(), std::io::Error>(()));
    log_connection_result(Err(std::io::Error::other("boom")));
}

#[tokio::test]
async fn successful_accepts_reset_transient_failure_counts() {
    let (address, accepted_stream, client) = accept_test_stream().await;
    let mut failures = 3usize;
    let mut connections = tokio::task::JoinSet::new();
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let acceptor = test_tls_acceptor(address);
    let router = test_router();
    let shutdown = std::future::pending::<()>();
    tokio::pin!(shutdown);

    let action = handle_accept_result(
        Ok((accepted_stream, address)),
        &mut failures,
        AcceptContext {
            connections: &mut connections,
            tls_acceptor: &acceptor,
            app: &router,
            shutdown_rx: &shutdown_rx,
            shutdown_tx: &shutdown_tx,
        },
        shutdown.as_mut(),
    )
    .await
    .expect("successful accepts should continue serving");

    assert_eq!(action, AcceptLoopAction::Continue);
    assert_eq!(failures, 0);

    drop(client);
    timeout(Duration::from_secs(1), connections.join_next())
        .await
        .expect("the spawned TLS task should observe the failed handshake");
}

#[tokio::test]
async fn transient_accept_failures_retry_after_backoff() {
    let mut failures = 0usize;
    let mut connections = tokio::task::JoinSet::new();
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let acceptor = loopback_test_acceptor();
    let router = test_router();
    let shutdown = std::future::pending::<()>();
    tokio::pin!(shutdown);

    let action = handle_accept_result(
        Err(std::io::Error::from(std::io::ErrorKind::WouldBlock)),
        &mut failures,
        AcceptContext {
            connections: &mut connections,
            tls_acceptor: &acceptor,
            app: &router,
            shutdown_rx: &shutdown_rx,
            shutdown_tx: &shutdown_tx,
        },
        shutdown.as_mut(),
    )
    .await
    .expect("retryable accept errors should not fail serving");

    assert_eq!(action, AcceptLoopAction::Continue);
    assert_eq!(failures, 1);
}

#[tokio::test]
async fn transient_accept_failures_break_when_shutdown_arrives() {
    let mut failures = 0usize;
    let mut connections = tokio::task::JoinSet::new();
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let acceptor = loopback_test_acceptor();
    let router = test_router();
    let shutdown = std::future::ready(());
    tokio::pin!(shutdown);

    let action = handle_accept_result(
        Err(std::io::Error::from(std::io::ErrorKind::WouldBlock)),
        &mut failures,
        AcceptContext {
            connections: &mut connections,
            tls_acceptor: &acceptor,
            app: &router,
            shutdown_rx: &shutdown_rx,
            shutdown_tx: &shutdown_tx,
        },
        shutdown.as_mut(),
    )
    .await
    .expect("shutdown during backoff should stop serving cleanly");

    assert_eq!(action, AcceptLoopAction::Break);
    assert_eq!(failures, 1);
}

#[tokio::test]
async fn too_many_transient_accept_failures_stop_serving() {
    let mut failures = MAX_CONSECUTIVE_TRANSIENT_ACCEPT_ERRORS;
    let mut connections = tokio::task::JoinSet::new();
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let acceptor = loopback_test_acceptor();
    let router = test_router();
    let shutdown = std::future::pending::<()>();
    tokio::pin!(shutdown);

    let error = handle_accept_result(
        Err(std::io::Error::from(std::io::ErrorKind::WouldBlock)),
        &mut failures,
        AcceptContext {
            connections: &mut connections,
            tls_acceptor: &acceptor,
            app: &router,
            shutdown_rx: &shutdown_rx,
            shutdown_tx: &shutdown_tx,
        },
        shutdown.as_mut(),
    )
    .await
    .expect_err("too many retryable failures should stop serving");

    assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
    assert_eq!(failures, MAX_CONSECUTIVE_TRANSIENT_ACCEPT_ERRORS + 1);
}

#[tokio::test]
async fn fatal_accept_failures_stop_serving_immediately() {
    let mut failures = 0usize;
    let mut connections = tokio::task::JoinSet::new();
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let acceptor = loopback_test_acceptor();
    let router = test_router();
    let shutdown = std::future::pending::<()>();
    tokio::pin!(shutdown);

    let error = handle_accept_result(
        Err(std::io::Error::other("boom")),
        &mut failures,
        AcceptContext {
            connections: &mut connections,
            tls_acceptor: &acceptor,
            app: &router,
            shutdown_rx: &shutdown_rx,
            shutdown_tx: &shutdown_tx,
        },
        shutdown.as_mut(),
    )
    .await
    .expect_err("fatal accept errors should stop serving");

    assert_eq!(error.kind(), std::io::ErrorKind::Other);
    assert_eq!(failures, 0);
}

#[tokio::test]
async fn spawned_connection_tasks_handle_failed_tls_handshakes() {
    let (address, stream, client) = accept_test_stream().await;
    let mut connections = tokio::task::JoinSet::new();
    let (_, shutdown_rx) = tokio::sync::watch::channel(false);

    spawn_test_connection_task(&mut connections, address, shutdown_rx, stream);

    drop(client);
    timeout(Duration::from_secs(1), connections.join_next())
        .await
        .expect("failed TLS handshakes should finish promptly");
}

#[tokio::test]
async fn spawned_connection_tasks_honor_shutdown_signals() {
    let (address, stream, _client, request) = prepare_shutdown_test_connection().await;
    let mut connections = tokio::task::JoinSet::new();
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    spawn_test_connection_task(&mut connections, address, shutdown_rx, stream);

    request
        .await
        .expect("the client request should finish successfully");
    shutdown_tx
        .send(true)
        .expect("shutdown signals should be broadcast");

    timeout(Duration::from_secs(1), connections.join_next())
        .await
        .expect("shutdown should drain active connections");
}

#[tokio::test]
async fn draining_empty_connection_tasks_returns_immediately() {
    let mut connections = tokio::task::JoinSet::new();

    timeout(
        Duration::from_millis(50),
        drain_connection_tasks(&mut connections),
    )
    .await
    .expect("empty connection sets should not wait for the shutdown grace period");
}

#[tokio::test]
async fn draining_pending_connection_tasks_aborts_after_the_grace_period() {
    let mut connections = tokio::task::JoinSet::new();
    connections.spawn(std::future::pending::<()>());

    timeout(
        Duration::from_secs(2),
        drain_connection_tasks(&mut connections),
    )
    .await
    .expect("pending connections should be aborted after the shutdown grace period");

    assert!(connections.is_empty());
}

#[tokio::test]
async fn draining_connection_tasks_tolerates_aborted_join_handles() {
    let mut connections = tokio::task::JoinSet::new();
    connections.spawn(async {
        panic!("boom");
    });

    timeout(
        Duration::from_secs(1),
        drain_connection_tasks(&mut connections),
    )
    .await
    .expect("aborted connection tasks should still drain promptly");

    assert!(connections.is_empty());
}

#[tokio::test]
async fn graceful_connection_shutdown_returns_after_success() {
    let future = std::future::ready(Ok::<(), std::io::Error>(()));
    tokio::pin!(future);

    finish_connection_after_shutdown(future.as_mut()).await;
}

#[tokio::test]
async fn graceful_connection_shutdown_handles_connection_errors() {
    let future = std::future::ready(Err::<(), _>(std::io::Error::other("boom")));
    tokio::pin!(future);

    finish_connection_after_shutdown(future.as_mut()).await;
}

#[tokio::test]
async fn graceful_connection_shutdown_times_out_pending_connections() {
    let future = std::future::pending::<std::io::Result<()>>();
    tokio::pin!(future);

    timeout(
        Duration::from_secs(1),
        finish_connection_after_shutdown(future.as_mut()),
    )
    .await
    .expect("pending connections should stop after the graceful shutdown deadline");
}

#[test]
fn loopback_tls_acceptor_supports_additional_loopback_addresses() {
    let address = "127.0.0.2:8443"
        .parse()
        .expect("loopback socket addresses should parse");

    build_loopback_tls_acceptor(address).expect("loopback certificates should build");
}

#[test]
fn transient_accept_errors_cover_standard_retryable_kinds() {
    for kind in [
        std::io::ErrorKind::ConnectionAborted,
        std::io::ErrorKind::Interrupted,
        std::io::ErrorKind::TimedOut,
        std::io::ErrorKind::WouldBlock,
    ] {
        assert!(accept_error_is_transient(&std::io::Error::from(kind)));
    }
}

#[cfg(unix)]
#[test]
fn transient_accept_errors_cover_retryable_errno_values() {
    for errno in [
        libc::ECONNABORTED,
        libc::EINTR,
        libc::EMFILE,
        libc::ENFILE,
        libc::ENOBUFS,
        libc::ENOMEM,
    ] {
        assert!(accept_error_is_transient(
            &std::io::Error::from_raw_os_error(errno)
        ));
    }
}

#[test]
fn transient_accept_errors_reject_fatal_errors() {
    assert!(!accept_error_is_transient(&std::io::Error::other("boom")));
}

#[test]
fn session_store_errors_map_to_matching_http_categories() {
    let cases = [
        (
            SessionStoreError::NotFound,
            StatusCode::NOT_FOUND,
            "session not found",
        ),
        (
            SessionStoreError::Forbidden,
            StatusCode::FORBIDDEN,
            "session owner mismatch",
        ),
        (
            SessionStoreError::Closed,
            StatusCode::CONFLICT,
            "session already closed",
        ),
        (
            SessionStoreError::EmptyPrompt,
            StatusCode::BAD_REQUEST,
            "prompt must not be empty",
        ),
        (
            SessionStoreError::PermissionNotFound,
            StatusCode::NOT_FOUND,
            "permission request not found",
        ),
        (
            SessionStoreError::SessionCapReached,
            StatusCode::TOO_MANY_REQUESTS,
            "session cap reached for principal",
        ),
    ];

    for (source, expected_status, expected_message) in cases {
        let error: AppError = source.into();

        assert_eq!(error.status_code(), expected_status);
        assert_eq!(error.message(), expected_message);
    }
}

#[tokio::test]
async fn injected_reply_provider_handles_prompt_dispatch() {
    let store = Arc::new(SessionStore::new(4));
    let state = AppState::with_dependencies(
        store.clone(),
        Arc::new(StaticReplyProvider {
            reply: "injected reply".to_string(),
        }),
    );
    let session = store
        .create_session("alice")
        .await
        .expect("session creation should succeed");
    let _ = post_message(
        State(state),
        Path(session.id.clone()),
        bearer_principal("alice"),
        Json(PromptRequest {
            text: "hello".to_string(),
        }),
    )
    .await
    .expect("prompt submission should succeed");

    let history = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let history = store
                .session_history("alice", &session.id)
                .await
                .expect("session history should load");
            if history.len() == 2 {
                return history;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("assistant reply should be recorded");

    assert_eq!(history[1].text, "injected reply");
}

#[tokio::test]
async fn create_session_seeds_startup_hints_when_enabled() {
    let store = Arc::new(SessionStore::new(4));
    let state = AppState {
        store: store.clone(),
        workspace_repository: new_ephemeral_workspace_repository(),
        reply_provider: Arc::new(StartupHintProvider {
            hint: "bundled mock verification ready".to_string(),
        }),
        startup_hints: true,
        frontend_dist: None,
    };
    let response = create_session(State(state), bearer_principal("alice"))
        .await
        .expect("session creation should succeed");

    assert_eq!(response.0, StatusCode::CREATED);
    assert_eq!(response.1.0.session.messages.len(), 1);
    assert_eq!(
        response.1.0.session.messages[0].text,
        "bundled mock verification ready"
    );
}

#[tokio::test]
async fn create_session_skips_startup_hints_when_disabled() {
    let store = Arc::new(SessionStore::new(4));
    let state = AppState {
        store,
        workspace_repository: new_ephemeral_workspace_repository(),
        reply_provider: Arc::new(StartupHintProvider {
            hint: "should stay hidden".to_string(),
        }),
        startup_hints: false,
        frontend_dist: None,
    };
    let response = create_session(State(state), bearer_principal("alice"))
        .await
        .expect("session creation should succeed");

    assert_eq!(response.0, StatusCode::CREATED);
    assert!(response.1.0.session.messages.is_empty());
}

#[tokio::test]
async fn create_session_keeps_sessions_without_primeable_startup_hints() {
    #[derive(Debug)]
    struct NoStartupHintProvider;

    impl ReplyProvider for NoStartupHintProvider {
        fn request_reply<'a>(&'a self, _turn: TurnHandle) -> ReplyFuture<'a> {
            Box::pin(async { Ok(ReplyResult::NoOutput) })
        }
    }

    let store = Arc::new(SessionStore::new(4));
    let state = AppState {
        store,
        workspace_repository: new_ephemeral_workspace_repository(),
        reply_provider: Arc::new(NoStartupHintProvider),
        startup_hints: true,
        frontend_dist: None,
    };
    let response = create_session(State(state), bearer_principal("alice"))
        .await
        .expect("session creation should succeed");

    assert_eq!(response.0, StatusCode::CREATED);
    assert!(response.1.0.session.messages.is_empty());
}

#[tokio::test]
async fn create_session_rolls_back_when_startup_hints_fail() {
    let store = Arc::new(SessionStore::new(1));
    let forgotten_sessions = StdArc::new(Mutex::new(Vec::new()));
    let state = AppState {
        store: store.clone(),
        workspace_repository: new_ephemeral_workspace_repository(),
        reply_provider: Arc::new(FailingStartupHintProvider {
            forgotten_sessions: forgotten_sessions.clone(),
        }),
        startup_hints: true,
        frontend_dist: None,
    };
    let error = create_session(State(state), bearer_principal("alice"))
        .await
        .expect_err("failed startup hint priming should fail the request");

    assert!(
        matches!(error, AppError::Internal(message) if message.contains("startup hint priming failed"))
    );
    let rolled_back_session_id = forgotten_sessions
        .lock()
        .expect("cleanup tracking should not poison")
        .first()
        .cloned()
        .expect("failed priming should forget the provisional session");
    let snapshot_error = store
        .session_snapshot("alice", &rolled_back_session_id)
        .await
        .expect_err("rolled back sessions should be removed");
    assert_eq!(snapshot_error, SessionStoreError::NotFound);
    store
        .create_session("alice")
        .await
        .expect("rollback should free the session cap");
}

#[tokio::test]
async fn create_session_reports_rollback_failures() {
    let store = Arc::new(SessionStore::new(1));
    let forgotten_sessions = StdArc::new(Mutex::new(Vec::new()));
    let state = AppState {
        store: store.clone(),
        workspace_repository: new_ephemeral_workspace_repository(),
        reply_provider: Arc::new(RollbackFailingStartupHintProvider {
            store: store.clone(),
            owner: "alice".to_string(),
            forgotten_sessions: forgotten_sessions.clone(),
        }),
        startup_hints: true,
        frontend_dist: None,
    };
    let error = create_session(State(state), bearer_principal("alice"))
        .await
        .expect_err("rollback failures should surface as internal errors");

    assert!(matches!(
        error,
        AppError::Internal(message)
            if message.contains("startup hint priming failed")
                && message.contains("session rollback failed: session not found")
    ));
    assert_eq!(
        forgotten_sessions
            .lock()
            .expect("cleanup tracking should not poison")
            .len(),
        1
    );
}

#[tokio::test]
async fn closing_sessions_notifies_reply_provider_cleanup() {
    let store = Arc::new(SessionStore::new(4));
    let forgotten_sessions = StdArc::new(Mutex::new(Vec::new()));
    let state = AppState::with_dependencies(
        store.clone(),
        Arc::new(TrackingReplyProvider {
            forgotten_sessions: forgotten_sessions.clone(),
        }),
    );
    let session = store
        .create_session("alice")
        .await
        .expect("session creation should succeed");
    let response = close_session(
        State(state),
        Path(session.id.clone()),
        bearer_principal("alice"),
    )
    .await
    .expect("closing the session should succeed");

    assert_eq!(response.0.session.id, session.id);
    assert_eq!(
        forgotten_sessions
            .lock()
            .expect("cleanup tracking should not poison")
            .as_slice(),
        [session.id]
    );
}

#[tokio::test]
async fn legacy_session_routes_persist_owner_scoped_metadata() {
    let context = metadata_test_context().await;
    assert_session_routes_persist_owner_scoped_metadata(&context).await;
}

#[tokio::test]
async fn browser_session_routes_persist_owner_scoped_metadata() {
    let context = browser_metadata_test_context().await;
    assert_session_routes_persist_owner_scoped_metadata(&context).await;
}

#[tokio::test]
async fn browser_session_writes_require_an_authenticated_account() {
    let context = browser_metadata_test_context().await;
    let (session, _) = create_persisted_session(&context).await;
    let replacement_admin = context
        .workspace_repository
        .create_local_account("backup-admin", "password123", true)
        .await
        .expect("creating a replacement admin should succeed");
    let invalidated_browser_sessions = context
        .workspace_repository
        .delete_local_account(&context.user.user_id, &replacement_admin.user_id)
        .await
        .expect("deleting the account should invalidate its browser sessions");

    assert_eq!(
        invalidated_browser_sessions,
        vec![context.live_owner_id.clone()]
    );

    let error = post_message(
        State(context.state.clone()),
        Path(session.id.clone()),
        context.principal.clone(),
        Json(PromptRequest {
            text: "should not run".to_string(),
        }),
    )
    .await
    .expect_err("deleted browser accounts should not post prompts");

    assert!(
        matches!(error, AppError::Unauthorized(message) if message == "account authentication required")
    );
    let snapshot = context
        .store
        .session_snapshot(&context.live_owner_id, &session.id)
        .await
        .expect("live session should still exist before in-memory invalidation");
    assert!(snapshot.messages.is_empty());
}

#[tokio::test]
async fn create_session_scrubs_workspace_store_failures() {
    let state = AppState::with_workspace_repository(
        Arc::new(SessionStore::new(4)),
        Arc::new(FailingWorkspaceStore::new("db path leaked")),
        Arc::new(TrackingReplyProvider {
            forgotten_sessions: StdArc::new(Mutex::new(Vec::new())),
        }),
    );

    let error = create_session(State(state), bearer_principal("alice"))
        .await
        .expect_err("workspace store failures should surface as internal errors");

    assert!(matches!(error, AppError::Internal(message) if message == "internal server error"));
}

#[test]
fn app_state_build_errors_format_and_expose_sources() {
    let reply_error = AppStateBuildError::from(MockClientError::TurnRuntime {
        message: "reply provider failed".to_string(),
    });
    let workspace_error = AppStateBuildError::from(WorkspaceStoreError::Database(
        "workspace store failed".to_string(),
    ));

    assert_eq!(
        reply_error.to_string(),
        "coordinating the prompt turn failed: reply provider failed"
    );
    assert_eq!(
        std::error::Error::source(&reply_error)
            .expect("reply provider sources should exist")
            .to_string(),
        "coordinating the prompt turn failed: reply provider failed"
    );
    assert_eq!(workspace_error.to_string(), "workspace store failed");
    assert_eq!(
        std::error::Error::source(&workspace_error)
            .expect("workspace store sources should exist")
            .to_string(),
        "workspace store failed"
    );
}

#[test]
fn app_state_debug_reports_public_fields() {
    let state = AppState::with_workspace_repository(
        Arc::new(SessionStore::new(4)),
        metadata_test_workspace_store(),
        Arc::new(TrackingReplyProvider {
            forgotten_sessions: StdArc::new(Mutex::new(Vec::new())),
        }),
    );
    let debug = format!("{state:?}");

    assert!(debug.contains("AppState"));
    assert!(debug.contains("startup_hints"));
    assert!(debug.contains("frontend_dist"));
}

#[test]
fn workspace_store_initialization_failures_map_into_app_state_build_errors() {
    let blocking_path = std::env::temp_dir().join(format!(
        "acp-web-backend-state-blocker-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let cleanup_path = blocking_path.clone();
    std::fs::write(&blocking_path, "blocker").expect("creating the blocking file should succeed");

    let error = SqliteWorkspaceRepository::new(blocking_path.join("db.sqlite"))
        .map_err(AppStateBuildError::from)
        .expect_err("invalid state roots should fail");

    assert!(matches!(error, AppStateBuildError::WorkspaceStore(_)));
    let _ = std::fs::remove_file(cleanup_path);
}

#[tokio::test]
async fn owner_context_surfaces_workspace_storage_failures() {
    let state = AppState::with_workspace_repository(
        Arc::new(SessionStore::new(4)),
        Arc::new(FailingWorkspaceStore::new("materialization unavailable")),
        Arc::new(TrackingReplyProvider {
            forgotten_sessions: StdArc::new(Mutex::new(Vec::new())),
        }),
    );
    let principal =
        authorize_request(&bearer_headers("alice"), true).expect("bearer headers should authorize");

    let error = state
        .owner_context(principal)
        .await
        .expect_err("owner context should surface workspace storage failures");
    assert!(matches!(error, AppError::Internal(message) if message == "internal server error"));
}

#[tokio::test]
async fn persist_session_metadata_best_effort_swallows_workspace_storage_errors() {
    let state = AppState::with_workspace_repository(
        Arc::new(SessionStore::new(4)),
        Arc::new(FailingWorkspaceStore::new("metadata unavailable")),
        Arc::new(TrackingReplyProvider {
            forgotten_sessions: StdArc::new(Mutex::new(Vec::new())),
        }),
    );

    persist_session_metadata_best_effort(
        &state,
        &sample_user_record(),
        &sample_snapshot("s_best_effort"),
        true,
        None,
        "test",
    )
    .await;
}

#[tokio::test]
async fn persist_prompt_snapshot_best_effort_swallows_snapshot_failures() {
    let state = AppState::with_workspace_repository(
        Arc::new(SessionStore::new(4)),
        metadata_test_workspace_store(),
        Arc::new(TrackingReplyProvider {
            forgotten_sessions: StdArc::new(Mutex::new(Vec::new())),
        }),
    );
    persist_prompt_snapshot_best_effort(
        &state,
        &sample_user_record(),
        "s_missing",
        Err(SessionStoreError::NotFound),
    )
    .await;
}

#[tokio::test]
async fn create_session_reports_metadata_rollback_failures() {
    let store = Arc::new(SessionStore::new(1));
    let state = AppState {
        store: store.clone(),
        workspace_repository: Arc::new(RollbackFailingMetadataWorkspaceStore::new(
            store,
            "alice",
            "metadata write failed",
            true,
        )),
        reply_provider: Arc::new(TrackingReplyProvider {
            forgotten_sessions: StdArc::new(Mutex::new(Vec::new())),
        }),
        startup_hints: false,
        frontend_dist: None,
    };

    let error = create_session(State(state), bearer_principal("alice"))
        .await
        .expect_err("metadata rollback failures should surface as internal errors");

    assert!(matches!(
        error,
        AppError::Internal(message)
            if message.contains("internal server error")
                && message.contains("session rollback failed: session not found")
    ));
}

#[tokio::test]
async fn create_session_rolls_back_when_metadata_persistence_fails() {
    let store = Arc::new(SessionStore::new(1));
    let state = AppState {
        store: store.clone(),
        workspace_repository: Arc::new(RollbackFailingMetadataWorkspaceStore::new(
            store.clone(),
            "alice",
            "metadata write failed",
            false,
        )),
        reply_provider: Arc::new(TrackingReplyProvider {
            forgotten_sessions: StdArc::new(Mutex::new(Vec::new())),
        }),
        startup_hints: false,
        frontend_dist: None,
    };

    let error = create_session(State(state), bearer_principal("alice"))
        .await
        .expect_err("metadata persistence failures should roll back the session");

    assert!(matches!(error, AppError::Internal(message) if message == "internal server error"));
    let snapshot_error = store
        .session_snapshot("alice", "s_1")
        .await
        .expect_err("failed creations should be rolled back");
    assert_eq!(snapshot_error, SessionStoreError::NotFound);
}

#[tokio::test]
async fn rename_session_keeps_working_when_workspace_materialization_fails() {
    let store = Arc::new(SessionStore::new(4));
    let session = store
        .create_session("alice")
        .await
        .expect("session creation should succeed");
    let state = AppState::with_workspace_repository(
        store,
        Arc::new(FailingWorkspaceStore::new("metadata unavailable")),
        Arc::new(TrackingReplyProvider {
            forgotten_sessions: StdArc::new(Mutex::new(Vec::new())),
        }),
    );

    let renamed = rename_session(
        State(state),
        Path(session.id.clone()),
        bearer_principal("alice"),
        Json(RenameSessionRequest {
            title: "Renamed while metadata failed".to_string(),
        }),
    )
    .await
    .expect("live session rename should still succeed");

    assert_eq!(renamed.0.session.title, "Renamed while metadata failed");
}

#[tokio::test]
async fn post_message_keeps_working_when_workspace_materialization_fails() {
    let store = Arc::new(SessionStore::new(4));
    let session = store
        .create_session("alice")
        .await
        .expect("session creation should succeed");
    let response = post_message(
        State(failing_workspace_state(store)),
        Path(session.id.clone()),
        bearer_principal("alice"),
        Json(PromptRequest {
            text: "hello from a degraded metadata store".to_string(),
        }),
    )
    .await
    .expect("live prompt submission should still succeed");

    assert!(response.0.accepted);
}

#[tokio::test]
async fn close_session_keeps_working_when_workspace_materialization_fails() {
    let store = Arc::new(SessionStore::new(4));
    let session = store
        .create_session("alice")
        .await
        .expect("session creation should succeed");
    let response = close_session(
        State(failing_workspace_state(store)),
        Path(session.id.clone()),
        bearer_principal("alice"),
    )
    .await
    .expect("live session close should still succeed");

    assert_eq!(response.0.session.status, SessionStatus::Closed);
}

#[tokio::test]
async fn delete_session_keeps_working_when_workspace_materialization_fails() {
    let store = Arc::new(SessionStore::new(4));
    let session = store
        .create_session("alice")
        .await
        .expect("session creation should succeed");
    let state = failing_workspace_state(store.clone());

    let response = delete_session(
        State(state),
        Path(session.id.clone()),
        bearer_principal("alice"),
    )
    .await
    .expect("live session deletion should still succeed");

    assert!(response.0.deleted);
    let snapshot_error = store
        .session_snapshot("alice", &session.id)
        .await
        .expect_err("deleted sessions should be removed from the live store");
    assert_eq!(snapshot_error, SessionStoreError::NotFound);
}

fn metadata_test_workspace_store() -> Arc<SqliteWorkspaceRepository> {
    Arc::new(
        SqliteWorkspaceRepository::new(
            std::env::temp_dir()
                .join(format!(
                    "acp-server-route-metadata-{}",
                    uuid::Uuid::new_v4().simple()
                ))
                .join("db.sqlite"),
        )
        .expect("workspace repository should initialize"),
    )
}

struct MetadataTestContext {
    store: Arc<SessionStore>,
    workspace_repository: Arc<SqliteWorkspaceRepository>,
    state: AppState,
    live_owner_id: String,
    principal: Extension<AuthenticatedPrincipal>,
    user: UserRecord,
}

async fn metadata_test_context() -> MetadataTestContext {
    let store = Arc::new(SessionStore::new(4));
    let workspace_repository = metadata_test_workspace_store();
    let state = AppState::with_workspace_repository(
        store.clone(),
        workspace_repository.clone(),
        Arc::new(TrackingReplyProvider {
            forgotten_sessions: StdArc::new(Mutex::new(Vec::new())),
        }),
    );
    let headers = bearer_headers("alice");
    let user = materialized_user_for_headers(workspace_repository.as_ref(), &headers).await;

    MetadataTestContext {
        store,
        workspace_repository,
        state,
        live_owner_id: "alice".to_string(),
        principal: bearer_principal("alice"),
        user,
    }
}

async fn browser_metadata_test_context() -> MetadataTestContext {
    let store = Arc::new(SessionStore::new(4));
    let workspace_repository = metadata_test_workspace_store();
    let state = AppState::with_workspace_repository(
        store.clone(),
        workspace_repository.clone(),
        Arc::new(TrackingReplyProvider {
            forgotten_sessions: StdArc::new(Mutex::new(Vec::new())),
        }),
    );
    let shell = app_entrypoint(HeaderMap::new()).await;
    let mut headers = browser_cookie_headers(&shell);
    let body = to_bytes(shell.into_body(), usize::MAX)
        .await
        .expect("entrypoint body should be readable");
    let body = String::from_utf8(body.to_vec()).expect("entrypoint body should be UTF-8");
    let csrf_token = extract_meta_content(&body, "acp-csrf-token");
    headers.insert("x-csrf-token", HeaderValue::from_str(&csrf_token).unwrap());
    let principal = authorize_request(&headers, true).expect("browser headers should authorize");
    let _registered = bootstrap_register(
        State(state.clone()),
        Extension(principal.clone()),
        Json(BootstrapRegistrationRequest {
            username: "admin".to_string(),
            password: "password123".to_string(),
        }),
    )
    .await
    .expect("bootstrap registration should succeed");
    let user = materialized_user_for_headers(workspace_repository.as_ref(), &headers).await;

    MetadataTestContext {
        store,
        workspace_repository,
        state,
        live_owner_id: principal.id.clone(),
        principal: Extension(principal),
        user,
    }
}

async fn assert_session_routes_persist_owner_scoped_metadata(context: &MetadataTestContext) {
    let (created, created_metadata) = create_persisted_session(context).await;

    assert_eq!(created_metadata.owner_user_id, context.user.user_id);
    assert_eq!(created_metadata.status, "active");
    assert!(!created_metadata.workspace_id.is_empty());

    let (renamed, renamed_metadata) = rename_persisted_session(context, &created.id).await;

    assert_eq!(renamed.title, "Renamed session");
    assert_eq!(renamed_metadata.title, "Renamed session");
    assert_eq!(renamed_metadata.workspace_id, created_metadata.workspace_id);
    assert_eq!(
        renamed_metadata.last_activity_at,
        created_metadata.last_activity_at
    );

    let active_metadata = post_message_and_load_metadata(context, &created.id).await;

    assert_eq!(active_metadata.status, "active");
    assert!(active_metadata.last_activity_at >= renamed_metadata.last_activity_at);

    let closed_metadata = close_session_and_load_metadata(context, &created.id).await;

    assert_eq!(closed_metadata.status, "closed");
    assert!(closed_metadata.closed_at.is_some());

    let deleted_metadata = delete_session_and_load_metadata(context, &created.id).await;

    assert_eq!(deleted_metadata.status, "deleted");
    assert!(deleted_metadata.deleted_at.is_some());
    let snapshot_error = context
        .store
        .session_snapshot(&context.live_owner_id, &created.id)
        .await
        .expect_err("deleted sessions should be removed from the live store");
    assert_eq!(snapshot_error, SessionStoreError::NotFound);
}

async fn create_persisted_session(
    context: &MetadataTestContext,
) -> (SessionSnapshot, SessionMetadataRecord) {
    let session = create_session(State(context.state.clone()), context.principal.clone())
        .await
        .expect("session creation should succeed")
        .1
        .0
        .session;
    let metadata = load_session_metadata_or_panic(
        context.workspace_repository.as_ref(),
        &context.user.user_id,
        &session.id,
        "created",
    )
    .await;

    (session, metadata)
}

async fn rename_persisted_session(
    context: &MetadataTestContext,
    session_id: &str,
) -> (SessionSnapshot, SessionMetadataRecord) {
    let session = rename_session(
        State(context.state.clone()),
        Path(session_id.to_string()),
        context.principal.clone(),
        Json(RenameSessionRequest {
            title: "Renamed session".to_string(),
        }),
    )
    .await
    .expect("session rename should succeed")
    .0
    .session;
    let metadata = load_session_metadata_or_panic(
        context.workspace_repository.as_ref(),
        &context.user.user_id,
        session_id,
        "renamed",
    )
    .await;

    (session, metadata)
}

async fn post_message_and_load_metadata(
    context: &MetadataTestContext,
    session_id: &str,
) -> SessionMetadataRecord {
    let _ = post_message(
        State(context.state.clone()),
        Path(session_id.to_string()),
        context.principal.clone(),
        Json(PromptRequest {
            text: "hello metadata".to_string(),
        }),
    )
    .await
    .expect("prompt submission should succeed");

    load_session_metadata_or_panic(
        context.workspace_repository.as_ref(),
        &context.user.user_id,
        session_id,
        "active",
    )
    .await
}

async fn close_session_and_load_metadata(
    context: &MetadataTestContext,
    session_id: &str,
) -> SessionMetadataRecord {
    let _ = close_session(
        State(context.state.clone()),
        Path(session_id.to_string()),
        context.principal.clone(),
    )
    .await
    .expect("session close should succeed");

    load_session_metadata_or_panic(
        context.workspace_repository.as_ref(),
        &context.user.user_id,
        session_id,
        "closed",
    )
    .await
}

async fn delete_session_and_load_metadata(
    context: &MetadataTestContext,
    session_id: &str,
) -> SessionMetadataRecord {
    let _ = delete_session(
        State(context.state.clone()),
        Path(session_id.to_string()),
        context.principal.clone(),
    )
    .await
    .expect("session deletion should succeed");

    load_session_metadata_or_panic(
        context.workspace_repository.as_ref(),
        &context.user.user_id,
        session_id,
        "deleted",
    )
    .await
}

fn bearer_headers(owner: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::AUTHORIZATION,
        format!("Bearer {owner}")
            .parse()
            .expect("authorization should parse"),
    );
    headers
}

fn bearer_principal(owner: &str) -> Extension<AuthenticatedPrincipal> {
    Extension(authorize_request(&bearer_headers(owner), false).expect("headers should authorize"))
}

fn browser_cookie_headers(response: &Response) -> HeaderMap {
    let mut headers = HeaderMap::new();
    let cookie_header = response
        .headers()
        .get_all(SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .filter_map(|cookie| cookie.split(';').next())
        .collect::<Vec<_>>()
        .join("; ");
    headers.insert(COOKIE, HeaderValue::from_str(&cookie_header).unwrap());
    headers
}

#[derive(Clone)]
struct BrowserAuthContext {
    headers: HeaderMap,
    principal: AuthenticatedPrincipal,
}

impl BrowserAuthContext {
    async fn spawn() -> Self {
        let shell = app_entrypoint(HeaderMap::new()).await;
        let headers = browser_cookie_headers(&shell);
        let body = to_bytes(shell.into_body(), usize::MAX)
            .await
            .expect("entrypoint body should be readable");
        let body = String::from_utf8(body.to_vec()).expect("entrypoint body should be utf-8");
        let csrf_token = extract_meta_content(&body, "acp-csrf-token");

        Self {
            principal: authorize_browser_headers(&headers, &csrf_token),
            headers,
        }
    }
}

fn auth_test_state() -> AppState {
    AppState::with_workspace_repository(
        Arc::new(SessionStore::new(4)),
        new_ephemeral_workspace_repository(),
        Arc::new(StaticReplyProvider {
            reply: "test reply".to_string(),
        }),
    )
}

fn tracking_auth_test_state() -> (AppState, StdArc<Mutex<Vec<String>>>) {
    let forgotten_sessions = StdArc::new(Mutex::new(Vec::new()));
    let state = AppState::with_workspace_repository(
        Arc::new(SessionStore::new(4)),
        new_ephemeral_workspace_repository(),
        Arc::new(TrackingReplyProvider {
            forgotten_sessions: forgotten_sessions.clone(),
        }),
    );
    (state, forgotten_sessions)
}

fn authorize_browser_headers(headers: &HeaderMap, csrf_token: &str) -> AuthenticatedPrincipal {
    let mut write_headers = headers.clone();
    write_headers.insert("x-csrf-token", HeaderValue::from_str(csrf_token).unwrap());
    authorize_request(&write_headers, true).expect("browser headers should authorize")
}

async fn bootstrap_admin_account(state: &AppState, browser: &BrowserAuthContext) {
    let _registered = bootstrap_register(
        State(state.clone()),
        Extension(browser.principal.clone()),
        Json(BootstrapRegistrationRequest {
            username: "admin".to_string(),
            password: "password123".to_string(),
        }),
    )
    .await
    .expect("bootstrap registration should succeed");
}

async fn create_member_account(
    state: &AppState,
    admin_browser: &BrowserAuthContext,
    username: &str,
    password: &str,
) -> acp_contracts::LocalAccount {
    create_account(
        State(state.clone()),
        Extension(admin_browser.principal.clone()),
        Json(acp_contracts::CreateAccountRequest {
            username: username.to_string(),
            password: password.to_string(),
            is_admin: false,
        }),
    )
    .await
    .expect("member account creation should succeed")
    .1
    .0
    .account
}

async fn sign_in_browser_account(
    state: &AppState,
    browser: &BrowserAuthContext,
    username: &str,
    password: &str,
) -> acp_contracts::LocalAccount {
    sign_in(
        State(state.clone()),
        Extension(browser.principal.clone()),
        Json(SignInRequest {
            username: username.to_string(),
            password: password.to_string(),
        }),
    )
    .await
    .expect("sign-in should succeed")
    .0
    .account
}

fn extract_meta_content(document: &str, name: &str) -> String {
    let name_needle = format!(r#"name="{name}""#);
    let tag = document
        .lines()
        .find(|line| line.contains("<meta ") && line.contains(&name_needle))
        .expect("meta tag should exist")
        .trim();
    let content_start = tag.find(r#"content=""#).unwrap() + r#"content=""#.len();
    let content_end = tag[content_start..].find('"').unwrap() + content_start;
    tag[content_start..content_end].to_string()
}

async fn materialized_user_for_headers(
    workspace_store: &SqliteWorkspaceRepository,
    headers: &HeaderMap,
) -> UserRecord {
    let principal = authorize_request(headers, true).expect("headers should authorize");
    workspace_store
        .materialize_user(&principal)
        .await
        .expect("principal materialization should be stable")
}

async fn load_session_metadata_or_panic(
    workspace_store: &SqliteWorkspaceRepository,
    user_id: &str,
    session_id: &str,
    stage: &str,
) -> SessionMetadataRecord {
    workspace_store
        .load_session_metadata(user_id, session_id)
        .await
        .unwrap_or_else(|_| panic!("{stage} session metadata should load"))
        .unwrap_or_else(|| panic!("{stage} session metadata should exist"))
}

fn failing_workspace_state(store: Arc<SessionStore>) -> AppState {
    AppState::with_workspace_repository(
        store,
        Arc::new(FailingWorkspaceStore::new("metadata unavailable")),
        Arc::new(TrackingReplyProvider {
            forgotten_sessions: StdArc::new(Mutex::new(Vec::new())),
        }),
    )
}

fn sample_user_record() -> UserRecord {
    let now = chrono::Utc::now();
    UserRecord {
        user_id: "u_test".to_string(),
        principal_kind: "bearer".to_string(),
        principal_subject: "durable-subject".to_string(),
        username: Some("admin".to_string()),
        password_hash: None,
        is_admin: true,
        created_at: now,
        last_seen_at: now,
        deleted_at: None,
    }
}

fn sample_snapshot(session_id: &str) -> SessionSnapshot {
    SessionSnapshot {
        id: session_id.to_string(),
        title: "Test session".to_string(),
        status: SessionStatus::Active,
        latest_sequence: 0,
        messages: Vec::new(),
        pending_permissions: Vec::new(),
    }
}

#[test]
fn workspace_store_errors_map_to_the_expected_app_errors() {
    let unauthorized: AppError = WorkspaceStoreError::Unauthorized("auth".to_string()).into();
    let not_found: AppError = WorkspaceStoreError::NotFound("missing".to_string()).into();
    let conflict: AppError = WorkspaceStoreError::Conflict("conflict".to_string()).into();
    let bad_request: AppError = WorkspaceStoreError::Validation("invalid".to_string()).into();

    assert!(matches!(
        unauthorized,
        AppError::Unauthorized(message) if message == "auth"
    ));
    assert!(matches!(
        not_found,
        AppError::NotFound(message) if message == "missing"
    ));
    assert!(matches!(
        conflict,
        AppError::Conflict(message) if message == "conflict"
    ));
    assert!(matches!(
        bad_request,
        AppError::BadRequest(message) if message == "invalid"
    ));
}

#[tokio::test]
async fn tracking_reply_provider_returns_no_output() {
    let store = SessionStore::new(4);
    let session = store
        .create_session("alice")
        .await
        .expect("session creation should succeed");
    let pending = store
        .submit_prompt("alice", &session.id, "hello".to_string())
        .await
        .expect("prompt submission should succeed");
    let provider = TrackingReplyProvider {
        forgotten_sessions: StdArc::new(Mutex::new(Vec::new())),
    };

    let reply = provider
        .request_reply(pending.turn_handle())
        .await
        .expect("tracking providers should return cleanly");

    assert_eq!(reply, ReplyResult::NoOutput);
}

#[tokio::test]
async fn slash_completion_handler_returns_catalog_entries_for_the_owner() {
    let store = Arc::new(SessionStore::new(4));
    let state = AppState::with_dependencies(
        store.clone(),
        Arc::new(TrackingReplyProvider {
            forgotten_sessions: StdArc::new(Mutex::new(Vec::new())),
        }),
    );
    let session = store
        .create_session("alice")
        .await
        .expect("session creation should succeed");
    let response = get_slash_completions(
        State(state),
        Query(SlashCompletionsQuery {
            session_id: session.id,
            prefix: "/he".to_string(),
        }),
        bearer_principal("alice"),
    )
    .await
    .expect("authorized slash completion queries should succeed");

    assert_eq!(response.0.candidates.len(), 1);
    assert_eq!(response.0.candidates[0].label, "/help");
}

#[tokio::test]
async fn slash_completion_handler_requires_bearer_authentication() {
    let store = Arc::new(SessionStore::new(4));
    let state = AppState::with_dependencies(
        store.clone(),
        Arc::new(TrackingReplyProvider {
            forgotten_sessions: StdArc::new(Mutex::new(Vec::new())),
        }),
    );
    let session = store
        .create_session("alice")
        .await
        .expect("session creation should succeed");

    let response = app(state)
        .oneshot(
            axum::http::Request::builder()
                .uri(format!(
                    "/api/v1/completions/slash?sessionId={}&prefix=%2F",
                    session.id
                ))
                .body(Body::empty())
                .expect("request building should succeed"),
        )
        .await
        .expect("router calls should complete");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[derive(Debug)]
struct StaticReplyProvider {
    reply: String,
}

impl ReplyProvider for StaticReplyProvider {
    fn request_reply<'a>(&'a self, _turn: TurnHandle) -> ReplyFuture<'a> {
        let reply = self.reply.clone();
        Box::pin(async move { Ok(ReplyResult::Reply(reply)) })
    }
}

#[derive(Debug)]
struct TrackingReplyProvider {
    forgotten_sessions: StdArc<Mutex<Vec<String>>>,
}

impl ReplyProvider for TrackingReplyProvider {
    fn request_reply<'a>(&'a self, _turn: TurnHandle) -> ReplyFuture<'a> {
        Box::pin(async { Ok(ReplyResult::NoOutput) })
    }

    fn forget_session(&self, session_id: &str) {
        self.forgotten_sessions
            .lock()
            .expect("cleanup tracking should not poison")
            .push(session_id.to_string());
    }
}

#[derive(Debug)]
struct FailingWorkspaceStore {
    error: WorkspaceStoreError,
}

impl FailingWorkspaceStore {
    fn new(message: &str) -> Self {
        Self {
            error: WorkspaceStoreError::Database(message.to_string()),
        }
    }
}

#[derive(Debug)]
struct RollbackFailingMetadataWorkspaceStore {
    store: Arc<SessionStore>,
    live_owner: String,
    user: UserRecord,
    error: WorkspaceStoreError,
    discard_before_fail: bool,
}

impl RollbackFailingMetadataWorkspaceStore {
    fn new(
        store: Arc<SessionStore>,
        live_owner: &str,
        message: &str,
        discard_before_fail: bool,
    ) -> Self {
        Self {
            store,
            live_owner: live_owner.to_string(),
            user: sample_user_record(),
            error: WorkspaceStoreError::Database(message.to_string()),
            discard_before_fail,
        }
    }
}

#[async_trait]
impl WorkspaceRepository for FailingWorkspaceStore {
    async fn materialize_user(
        &self,
        _principal: &AuthenticatedPrincipal,
    ) -> Result<UserRecord, WorkspaceStoreError> {
        Err(self.error.clone())
    }

    async fn bootstrap_workspace(
        &self,
        _owner_user_id: &str,
    ) -> Result<WorkspaceRecord, WorkspaceStoreError> {
        Err(self.error.clone())
    }

    async fn save_session_metadata(
        &self,
        _record: &SessionMetadataRecord,
    ) -> Result<(), WorkspaceStoreError> {
        Err(self.error.clone())
    }

    async fn persist_session_snapshot(
        &self,
        _owner_user_id: &str,
        _snapshot: &SessionSnapshot,
        _touch_activity: bool,
        _status_override: Option<&str>,
    ) -> Result<(), WorkspaceStoreError> {
        Err(self.error.clone())
    }

    async fn load_session_metadata(
        &self,
        _owner_user_id: &str,
        _session_id: &str,
    ) -> Result<Option<SessionMetadataRecord>, WorkspaceStoreError> {
        Err(self.error.clone())
    }

    async fn auth_status(
        &self,
        _browser_session_id: Option<&str>,
    ) -> Result<(bool, Option<UserRecord>), WorkspaceStoreError> {
        Err(self.error.clone())
    }

    async fn authenticate_browser_session(
        &self,
        _browser_session_id: &str,
    ) -> Result<Option<UserRecord>, WorkspaceStoreError> {
        Err(self.error.clone())
    }

    async fn bootstrap_local_account(
        &self,
        _browser_session_id: &str,
        _username: &str,
        _password: &str,
    ) -> Result<acp_contracts::LocalAccount, WorkspaceStoreError> {
        Err(self.error.clone())
    }

    async fn sign_in_local_account(
        &self,
        _browser_session_id: &str,
        _username: &str,
        _password: &str,
    ) -> Result<acp_contracts::LocalAccount, WorkspaceStoreError> {
        Err(self.error.clone())
    }

    async fn list_local_accounts(
        &self,
    ) -> Result<Vec<acp_contracts::LocalAccount>, WorkspaceStoreError> {
        Err(self.error.clone())
    }

    async fn create_local_account(
        &self,
        _username: &str,
        _password: &str,
        _is_admin: bool,
    ) -> Result<acp_contracts::LocalAccount, WorkspaceStoreError> {
        Err(self.error.clone())
    }

    async fn update_local_account(
        &self,
        _target_user_id: &str,
        _current_user_id: &str,
        _password: Option<&str>,
        _is_admin: Option<bool>,
    ) -> Result<acp_contracts::LocalAccount, WorkspaceStoreError> {
        Err(self.error.clone())
    }

    async fn delete_local_account(
        &self,
        _target_user_id: &str,
        _current_user_id: &str,
    ) -> Result<Vec<String>, WorkspaceStoreError> {
        Err(self.error.clone())
    }
}

#[async_trait]
impl WorkspaceRepository for RollbackFailingMetadataWorkspaceStore {
    async fn materialize_user(
        &self,
        _principal: &AuthenticatedPrincipal,
    ) -> Result<UserRecord, WorkspaceStoreError> {
        Ok(self.user.clone())
    }

    async fn bootstrap_workspace(
        &self,
        owner_user_id: &str,
    ) -> Result<WorkspaceRecord, WorkspaceStoreError> {
        Ok(WorkspaceRecord {
            workspace_id: "w_test".to_string(),
            owner_user_id: owner_user_id.to_string(),
            name: "Default workspace".to_string(),
            status: "active".to_string(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            deleted_at: None,
        })
    }

    async fn save_session_metadata(
        &self,
        _record: &SessionMetadataRecord,
    ) -> Result<(), WorkspaceStoreError> {
        Ok(())
    }

    async fn persist_session_snapshot(
        &self,
        _owner_user_id: &str,
        snapshot: &SessionSnapshot,
        _touch_activity: bool,
        _status_override: Option<&str>,
    ) -> Result<(), WorkspaceStoreError> {
        if self.discard_before_fail {
            let _ = self
                .store
                .discard_session(&self.live_owner, &snapshot.id)
                .await;
        }
        Err(self.error.clone())
    }

    async fn load_session_metadata(
        &self,
        _owner_user_id: &str,
        _session_id: &str,
    ) -> Result<Option<SessionMetadataRecord>, WorkspaceStoreError> {
        Ok(None)
    }

    async fn auth_status(
        &self,
        _browser_session_id: Option<&str>,
    ) -> Result<(bool, Option<UserRecord>), WorkspaceStoreError> {
        Ok((false, Some(self.user.clone())))
    }

    async fn authenticate_browser_session(
        &self,
        _browser_session_id: &str,
    ) -> Result<Option<UserRecord>, WorkspaceStoreError> {
        Ok(Some(self.user.clone()))
    }

    async fn bootstrap_local_account(
        &self,
        _browser_session_id: &str,
        _username: &str,
        _password: &str,
    ) -> Result<acp_contracts::LocalAccount, WorkspaceStoreError> {
        Ok(acp_contracts::LocalAccount {
            user_id: self.user.user_id.clone(),
            username: self
                .user
                .username
                .clone()
                .unwrap_or_else(|| "admin".to_string()),
            is_admin: self.user.is_admin,
            created_at: self.user.created_at,
        })
    }

    async fn sign_in_local_account(
        &self,
        _browser_session_id: &str,
        _username: &str,
        _password: &str,
    ) -> Result<acp_contracts::LocalAccount, WorkspaceStoreError> {
        self.bootstrap_local_account("", "", "").await
    }

    async fn list_local_accounts(
        &self,
    ) -> Result<Vec<acp_contracts::LocalAccount>, WorkspaceStoreError> {
        Ok(vec![acp_contracts::LocalAccount {
            user_id: self.user.user_id.clone(),
            username: self
                .user
                .username
                .clone()
                .unwrap_or_else(|| "admin".to_string()),
            is_admin: self.user.is_admin,
            created_at: self.user.created_at,
        }])
    }

    async fn create_local_account(
        &self,
        _username: &str,
        _password: &str,
        _is_admin: bool,
    ) -> Result<acp_contracts::LocalAccount, WorkspaceStoreError> {
        self.bootstrap_local_account("", "", "").await
    }

    async fn update_local_account(
        &self,
        _target_user_id: &str,
        _current_user_id: &str,
        _password: Option<&str>,
        _is_admin: Option<bool>,
    ) -> Result<acp_contracts::LocalAccount, WorkspaceStoreError> {
        self.bootstrap_local_account("", "", "").await
    }

    async fn delete_local_account(
        &self,
        _target_user_id: &str,
        _current_user_id: &str,
    ) -> Result<Vec<String>, WorkspaceStoreError> {
        Ok(Vec::new())
    }
}

#[derive(Debug)]
struct StartupHintProvider {
    hint: String,
}

impl ReplyProvider for StartupHintProvider {
    fn request_reply<'a>(&'a self, _turn: TurnHandle) -> ReplyFuture<'a> {
        Box::pin(async { Ok(ReplyResult::NoOutput) })
    }

    fn prime_session<'a>(
        &'a self,
        _session_id: &'a str,
    ) -> crate::mock_client::PrimeSessionFuture<'a> {
        let hint = self.hint.clone();
        Box::pin(async move { Ok(Some(hint)) })
    }
}

#[derive(Debug)]
struct FailingStartupHintProvider {
    forgotten_sessions: StdArc<Mutex<Vec<String>>>,
}

impl ReplyProvider for FailingStartupHintProvider {
    fn request_reply<'a>(&'a self, _turn: TurnHandle) -> ReplyFuture<'a> {
        Box::pin(async { Ok(ReplyResult::NoOutput) })
    }

    fn prime_session<'a>(
        &'a self,
        _session_id: &'a str,
    ) -> crate::mock_client::PrimeSessionFuture<'a> {
        Box::pin(async {
            Err(MockClientError::TurnRuntime {
                message: "startup hint priming failed".to_string(),
            })
        })
    }

    fn forget_session(&self, session_id: &str) {
        self.forgotten_sessions
            .lock()
            .expect("cleanup tracking should not poison")
            .push(session_id.to_string());
    }
}

#[derive(Debug)]
struct RollbackFailingStartupHintProvider {
    store: Arc<SessionStore>,
    owner: String,
    forgotten_sessions: StdArc<Mutex<Vec<String>>>,
}

impl ReplyProvider for RollbackFailingStartupHintProvider {
    fn request_reply<'a>(&'a self, _turn: TurnHandle) -> ReplyFuture<'a> {
        Box::pin(async { Ok(ReplyResult::NoOutput) })
    }

    fn prime_session<'a>(
        &'a self,
        session_id: &'a str,
    ) -> crate::mock_client::PrimeSessionFuture<'a> {
        let store = self.store.clone();
        let owner = self.owner.clone();
        let session_id = session_id.to_string();
        Box::pin(async move {
            store
                .discard_session(&owner, &session_id)
                .await
                .expect("the provisional session should exist before rollback");
            Err(MockClientError::TurnRuntime {
                message: "startup hint priming failed".to_string(),
            })
        })
    }

    fn forget_session(&self, session_id: &str) {
        self.forgotten_sessions
            .lock()
            .expect("cleanup tracking should not poison")
            .push(session_id.to_string());
    }
}

fn test_router() -> Router {
    app(test_state())
}

fn test_state() -> AppState {
    AppState::with_dependencies(
        Arc::new(SessionStore::new(4)),
        Arc::new(StaticReplyProvider {
            reply: "test reply".to_string(),
        }),
    )
}

async fn bind_test_listener() -> (tokio::net::TcpListener, std::net::SocketAddr) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("test listener should bind");
    let address = listener
        .local_addr()
        .expect("test listener should expose its address");
    (listener, address)
}

fn test_tls_acceptor(address: std::net::SocketAddr) -> TlsAcceptor {
    build_loopback_tls_acceptor(address).expect("loopback certificates should build")
}

fn loopback_test_acceptor() -> TlsAcceptor {
    test_tls_acceptor(
        "127.0.0.1:0"
            .parse()
            .expect("loopback socket addresses should parse"),
    )
}

fn spawn_test_connection_task(
    connections: &mut tokio::task::JoinSet<()>,
    address: std::net::SocketAddr,
    shutdown_rx: tokio::sync::watch::Receiver<bool>,
    stream: tokio::net::TcpStream,
) {
    spawn_connection_task(
        connections,
        test_tls_acceptor(address),
        test_router(),
        shutdown_rx,
        stream,
    );
}

async fn accept_test_stream() -> (
    std::net::SocketAddr,
    tokio::net::TcpStream,
    tokio::net::TcpStream,
) {
    let (listener, address) = bind_test_listener().await;
    let client = tokio::spawn(tokio::net::TcpStream::connect(address));
    let (stream, _) = listener
        .accept()
        .await
        .expect("accepted test streams should connect");
    let client = client
        .await
        .expect("client connect task should finish")
        .expect("client should connect");
    (address, stream, client)
}

async fn prepare_shutdown_test_connection() -> (
    std::net::SocketAddr,
    tokio::net::TcpStream,
    reqwest::Client,
    tokio::task::JoinHandle<()>,
) {
    let (listener, address) = bind_test_listener().await;
    let base_url = format!("https://{address}");
    let client = build_http_client_for_url(&base_url, Some(Duration::from_secs(1)))
        .expect("loopback clients should build");
    let request = tokio::spawn({
        let client = client.clone();
        let url = format!("{base_url}/healthz");
        async move {
            let response = client
                .get(url)
                .send()
                .await
                .expect("health requests should reach the server");
            response
                .error_for_status()
                .expect("health requests should succeed")
                .bytes()
                .await
                .expect("health responses should be readable");
        }
    });
    let (stream, _) = listener
        .accept()
        .await
        .expect("accepted test streams should connect");
    (address, stream, client, request)
}
