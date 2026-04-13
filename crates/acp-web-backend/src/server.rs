use std::{convert::Infallible, future::Future, pin::Pin, sync::Arc, time::Duration};

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
    mock_client::{MockClient, MockClientError},
    sessions::{PendingPrompt, SessionStore, SessionStoreError},
};

type SseStream = Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send>>;

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub session_cap: usize,
    pub mock_url: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            session_cap: 8,
            mock_url: "http://127.0.0.1:8090".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AppState {
    store: Arc<SessionStore>,
    mock_client: MockClient,
}

impl AppState {
    pub fn new(config: ServerConfig) -> Result<Self, MockClientError> {
        Ok(Self {
            store: Arc::new(SessionStore::new(config.session_cap)),
            mock_client: MockClient::new(config.mock_url)?,
        })
    }
}

pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/api/v1/sessions", post(create_session))
        .route("/api/v1/sessions/:session_id", get(get_session))
        .route(
            "/api/v1/sessions/:session_id/history",
            get(get_session_history),
        )
        .route(
            "/api/v1/sessions/:session_id/events",
            get(stream_session_events),
        )
        .route("/api/v1/sessions/:session_id/messages", post(post_message))
        .route("/api/v1/sessions/:session_id/close", post(close_session))
        .with_state(state)
}

pub async fn serve(listener: TcpListener, state: AppState) -> std::io::Result<()> {
    let address = listener.local_addr()?;
    info!("starting web backend on {address}");
    axum::serve(listener, app(state)).await
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
    dispatch_assistant_request(state.mock_client.clone(), pending);

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
) -> Result<Sse<SseStream>, AppError> {
    let principal = extract_principal(&headers)?;
    let (snapshot, receiver) = state
        .store
        .session_events(&principal.id, &session_id)
        .await?;

    let initial_event = stream::once(async move {
        Ok::<Event, Infallible>(to_sse_event(StreamEvent::snapshot(snapshot)))
    });
    let updates = BroadcastStream::new(receiver).filter_map(|result| async move {
        match result {
            Ok(event) => Some(Ok::<Event, Infallible>(to_sse_event(event))),
            Err(_) => None,
        }
    });

    let stream: SseStream = Box::pin(initial_event.chain(updates));

    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    ))
}

fn dispatch_assistant_request(mock_client: MockClient, pending: PendingPrompt) {
    let session_id = pending.session_id().to_string();
    let prompt = pending.prompt_text().to_string();

    tokio::spawn(async move {
        match mock_client.request_reply(&session_id, &prompt).await {
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
    let payload = match serde_json::to_string(&event) {
        Ok(payload) => payload,
        Err(_) => "{\"kind\":\"status\",\"message\":\"failed to serialize event\"}".to_string(),
    };

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

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, error) = match self {
            Self::Unauthorized(message) => (StatusCode::UNAUTHORIZED, message),
            Self::Forbidden(message) => (StatusCode::FORBIDDEN, message),
            Self::NotFound(message) => (StatusCode::NOT_FOUND, message),
            Self::BadRequest(message) => (StatusCode::BAD_REQUEST, message),
            Self::Conflict(message) => (StatusCode::CONFLICT, message),
            Self::TooManyRequests(message) => (StatusCode::TOO_MANY_REQUESTS, message),
            Self::Internal(message) => (StatusCode::INTERNAL_SERVER_ERROR, message),
        };

        (status, Json(ErrorResponse { error })).into_response()
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
