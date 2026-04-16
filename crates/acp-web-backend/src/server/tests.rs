use super::*;
use crate::mock_client::{ReplyFuture, ReplyResult};
use acp_app_support::build_http_client_for_url;
use axum::{
    body::to_bytes,
    http::{
        HeaderValue,
        header::{COOKIE, SET_COOKIE},
    },
};
use std::sync::{Arc as StdArc, Mutex};
use tokio::time::timeout;

#[test]
fn default_server_config_points_to_the_local_acp_server() {
    let config = ServerConfig::default();

    assert_eq!(config.session_cap, 8);
    assert_eq!(config.acp_server, "127.0.0.1:8090");
    assert!(!config.startup_hints);
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

    assert!(ct.starts_with("text/css"), "got: {ct}");
    assert!(!body.is_empty());
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
        std::fs::write(dir.join("acp-web-frontend-test.js"), b"// stub js loader")
            .expect("stub JS should be writable");
    }
    if include_wasm {
        std::fs::write(
            dir.join("acp-web-frontend-test_bg.wasm"),
            b"\x00asm\x01\x00\x00\x00", // minimal valid WASM header
        )
        .expect("stub WASM should be writable");
    }
    dir
}

fn write_temp_frontend_dist_with_unreadable_javascript() -> std::path::PathBuf {
    let dir = write_temp_frontend_dist_with(false, true);
    std::fs::create_dir(dir.join("acp-web-frontend-test.js"))
        .expect("stub unreadable JS directory should be creatable");
    dir
}

fn write_temp_frontend_dist_with_unreadable_wasm() -> std::path::PathBuf {
    let dir = write_temp_frontend_dist_with(true, false);
    std::fs::create_dir(dir.join("acp-web-frontend-test_bg.wasm"))
        .expect("stub unreadable WASM directory should be creatable");
    dir
}

fn test_state_with_frontend_dist(dist: std::path::PathBuf) -> AppState {
    AppState {
        store: Arc::new(SessionStore::new(1)),
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
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::AUTHORIZATION,
        "Bearer alice".parse().expect("authorization should parse"),
    );

    let _ = post_message(
        State(state),
        Path(session.id.clone()),
        headers,
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
        reply_provider: Arc::new(StartupHintProvider {
            hint: "bundled mock verification ready".to_string(),
        }),
        startup_hints: true,
        frontend_dist: None,
    };
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::AUTHORIZATION,
        "Bearer alice".parse().expect("authorization should parse"),
    );

    let response = create_session(State(state), headers)
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
        reply_provider: Arc::new(StartupHintProvider {
            hint: "should stay hidden".to_string(),
        }),
        startup_hints: false,
        frontend_dist: None,
    };
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::AUTHORIZATION,
        "Bearer alice".parse().expect("authorization should parse"),
    );

    let response = create_session(State(state), headers)
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
        reply_provider: Arc::new(NoStartupHintProvider),
        startup_hints: true,
        frontend_dist: None,
    };
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::AUTHORIZATION,
        "Bearer alice".parse().expect("authorization should parse"),
    );

    let response = create_session(State(state), headers)
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
        reply_provider: Arc::new(FailingStartupHintProvider {
            forgotten_sessions: forgotten_sessions.clone(),
        }),
        startup_hints: true,
        frontend_dist: None,
    };
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::AUTHORIZATION,
        "Bearer alice".parse().expect("authorization should parse"),
    );

    let error = create_session(State(state), headers)
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
        reply_provider: Arc::new(RollbackFailingStartupHintProvider {
            store: store.clone(),
            owner: "alice".to_string(),
            forgotten_sessions: forgotten_sessions.clone(),
        }),
        startup_hints: true,
        frontend_dist: None,
    };
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::AUTHORIZATION,
        "Bearer alice".parse().expect("authorization should parse"),
    );

    let error = create_session(State(state), headers)
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
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::AUTHORIZATION,
        "Bearer alice".parse().expect("authorization should parse"),
    );

    let response = close_session(State(state), Path(session.id.clone()), headers)
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
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::AUTHORIZATION,
        "Bearer alice".parse().expect("authorization should parse"),
    );

    let response = get_slash_completions(
        State(state),
        Query(SlashCompletionsQuery {
            session_id: session.id,
            prefix: "/he".to_string(),
        }),
        headers,
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

    let error = get_slash_completions(
        State(state),
        Query(SlashCompletionsQuery {
            session_id: session.id,
            prefix: "/".to_string(),
        }),
        HeaderMap::new(),
    )
    .await
    .expect_err("missing bearer auth must fail");

    assert!(matches!(error, AppError::Unauthorized(_)));
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
