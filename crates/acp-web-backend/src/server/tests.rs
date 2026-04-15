use super::*;
use crate::mock_client::{ReplyFuture, ReplyResult};
use std::sync::{Arc as StdArc, Mutex};

#[test]
fn default_server_config_points_to_the_local_acp_server() {
    let config = ServerConfig::default();

    assert_eq!(config.session_cap, 8);
    assert_eq!(config.acp_server, "127.0.0.1:8090");
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
    let missing: AppError = AuthError::MissingAuthorization.into();
    let invalid: AppError = AuthError::InvalidAuthorization.into();

    assert!(matches!(
        missing,
        AppError::Unauthorized(message) if message == "missing bearer token"
    ));
    assert!(matches!(
        invalid,
        AppError::Unauthorized(message) if message == "invalid bearer token"
    ));
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
