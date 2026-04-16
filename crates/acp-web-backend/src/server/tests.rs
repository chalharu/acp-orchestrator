use super::*;
use crate::mock_client::{ReplyFuture, ReplyResult};
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
async fn app_entrypoint_bootstraps_browser_cookies_and_markup() {
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

    assert!(body.contains("ACP Web MVP slice 0"));
    assert!(body.contains("name=\"acp-csrf-token\""));
    assert!(body.contains("/app/sessions/{id}"));
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
