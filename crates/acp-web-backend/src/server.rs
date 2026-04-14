use std::{convert::Infallible, future::Future, sync::Arc, time::Duration};

use acp_contracts::{
    CloseSessionResponse, CreateSessionResponse, ErrorResponse, HealthResponse, PromptRequest,
    PromptResponse, SessionHistoryResponse, StreamEvent,
};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post},
};
use futures_util::{Stream, StreamExt, stream};
use tokio::net::TcpListener;
use tokio_stream::wrappers::BroadcastStream;
use tracing::info;

use crate::{
    auth::{AuthError, extract_principal},
    mock_client::{MockClient, MockClientError, ReplyProvider},
    sessions::{PendingPrompt, SessionStore, SessionStoreError},
};

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub session_cap: usize,
    pub mock_address: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            session_cap: 8,
            mock_address: "127.0.0.1:8090".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AppState {
    store: Arc<SessionStore>,
    reply_provider: Arc<dyn ReplyProvider>,
}

impl AppState {
    pub fn new(config: ServerConfig) -> Result<Self, MockClientError> {
        Ok(Self::with_dependencies(
            Arc::new(SessionStore::new(config.session_cap)),
            Arc::new(MockClient::new(config.mock_address)?),
        ))
    }

    pub fn with_dependencies(
        store: Arc<SessionStore>,
        reply_provider: Arc<dyn ReplyProvider>,
    ) -> Self {
        Self {
            store,
            reply_provider,
        }
    }
}

pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/api/v1/sessions", post(create_session))
        .route("/api/v1/sessions/{session_id}", get(get_session))
        .route(
            "/api/v1/sessions/{session_id}/history",
            get(get_session_history),
        )
        .route(
            "/api/v1/sessions/{session_id}/events",
            get(stream_session_events),
        )
        .route("/api/v1/sessions/{session_id}/messages", post(post_message))
        .route("/api/v1/sessions/{session_id}/close", post(close_session))
        .with_state(state)
}

pub async fn serve_with_shutdown<F>(
    listener: TcpListener,
    state: AppState,
    shutdown: F,
) -> std::io::Result<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    let address = listener.local_addr()?;
    info!("starting web backend on {address}");
    axum::serve(listener, app(state))
        .with_graceful_shutdown(shutdown)
        .await
}

async fn healthz() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
    })
}

async fn create_session(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<(StatusCode, Json<CreateSessionResponse>), AppError> {
    let principal = extract_principal(&headers)?;
    let session = state.store.create_session(&principal.id).await?;

    Ok((StatusCode::CREATED, Json(CreateSessionResponse { session })))
}

async fn get_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<CreateSessionResponse>, AppError> {
    let principal = extract_principal(&headers)?;
    let session = state
        .store
        .session_snapshot(&principal.id, &session_id)
        .await?;

    Ok(Json(CreateSessionResponse { session }))
}

async fn get_session_history(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<SessionHistoryResponse>, AppError> {
    let principal = extract_principal(&headers)?;
    let messages = state
        .store
        .session_history(&principal.id, &session_id)
        .await?;

    Ok(Json(SessionHistoryResponse {
        session_id,
        messages,
    }))
}

async fn post_message(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<PromptRequest>,
) -> Result<Json<PromptResponse>, AppError> {
    let principal = extract_principal(&headers)?;
    let pending = state
        .store
        .submit_prompt(&principal.id, &session_id, request.text)
        .await?;
    dispatch_assistant_request(state.reply_provider.clone(), pending);

    Ok(Json(PromptResponse { accepted: true }))
}

async fn close_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<CloseSessionResponse>, AppError> {
    let principal = extract_principal(&headers)?;
    let session = state
        .store
        .close_session(&principal.id, &session_id)
        .await?;

    Ok(Json(CloseSessionResponse { session }))
}

async fn stream_session_events(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let principal = extract_principal(&headers)?;
    let (snapshot, receiver) = state
        .store
        .session_events(&principal.id, &session_id)
        .await?;

    let initial_event = stream::once(async move {
        Ok::<Event, Infallible>(to_sse_event(StreamEvent::snapshot(snapshot)))
    });
    let updates = BroadcastStream::new(receiver)
        .filter_map(|result| async move { result.ok().map(to_sse_event).map(Ok) });

    Ok(Sse::new(initial_event.chain(updates)).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    ))
}

fn dispatch_assistant_request(reply_provider: Arc<dyn ReplyProvider>, pending: PendingPrompt) {
    let session_id = pending.session_id().to_string();
    let prompt = pending.prompt_text().to_string();

    tokio::spawn(async move {
        match reply_provider.request_reply(&session_id, &prompt).await {
            Ok(reply) => pending.complete_with_reply(reply).await,
            Err(error) => {
                pending
                    .complete_with_status(format!("mock request failed: {error}"))
                    .await;
            }
        }
    });
}

fn to_sse_event(event: StreamEvent) -> Event {
    let sequence = event.sequence.to_string();
    let payload =
        serde_json::to_string(&event).expect("stream events should always serialize successfully");

    Event::default()
        .event(event.event_name())
        .id(sequence)
        .data(payload)
}

#[derive(Debug)]
pub enum AppError {
    Unauthorized(String),
    Forbidden(String),
    NotFound(String),
    BadRequest(String),
    Conflict(String),
    TooManyRequests(String),
    Internal(String),
}

impl AppError {
    fn status_code(&self) -> StatusCode {
        match self {
            Self::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::TooManyRequests(_) => StatusCode::TOO_MANY_REQUESTS,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn message(&self) -> &str {
        match self {
            Self::Unauthorized(message) => message,
            Self::Forbidden(message) => message,
            Self::NotFound(message) => message,
            Self::BadRequest(message) => message,
            Self::Conflict(message) => message,
            Self::TooManyRequests(message) => message,
            Self::Internal(message) => message,
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (
            self.status_code(),
            Json(ErrorResponse {
                error: self.message().to_string(),
            }),
        )
            .into_response()
    }
}

impl From<AuthError> for AppError {
    fn from(error: AuthError) -> Self {
        match error {
            AuthError::MissingAuthorization | AuthError::InvalidAuthorization => {
                Self::Unauthorized(error.message().to_string())
            }
        }
    }
}

impl From<SessionStoreError> for AppError {
    fn from(error: SessionStoreError) -> Self {
        match error {
            SessionStoreError::NotFound => Self::NotFound(error.message().to_string()),
            SessionStoreError::Forbidden => Self::Forbidden(error.message().to_string()),
            SessionStoreError::Closed => Self::Conflict(error.message().to_string()),
            SessionStoreError::EmptyPrompt => Self::BadRequest(error.message().to_string()),
            SessionStoreError::SessionCapReached => {
                Self::TooManyRequests(error.message().to_string())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock_client::ReplyFuture;

    #[test]
    fn default_server_config_points_to_the_local_mock() {
        let config = ServerConfig::default();

        assert_eq!(config.session_cap, 8);
        assert_eq!(config.mock_address, "127.0.0.1:8090");
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

    #[derive(Debug)]
    struct StaticReplyProvider {
        reply: String,
    }

    impl ReplyProvider for StaticReplyProvider {
        fn request_reply<'a>(&'a self, _session_id: &'a str, _prompt: &'a str) -> ReplyFuture<'a> {
            let reply = self.reply.clone();
            Box::pin(async move { Ok(reply) })
        }
    }
}
