use std::{convert::Infallible, future::Future, io, net::SocketAddr, sync::Arc, time::Duration};

use acp_contracts::{
    CancelTurnResponse, CloseSessionResponse, CreateSessionResponse, ErrorResponse, HealthResponse,
    PromptRequest, PromptResponse, ResolvePermissionRequest, ResolvePermissionResponse,
    SessionHistoryResponse, SessionListResponse, SessionSnapshot, SlashCompletionsResponse,
    StreamEvent,
};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{
        HeaderMap, HeaderValue, StatusCode,
        header::{CACHE_CONTROL, CONTENT_TYPE, REFERRER_POLICY, SET_COOKIE},
    },
    response::{
        Html, IntoResponse, Redirect, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post},
};
use futures_util::{Stream, StreamExt, stream};
use hyper::server::conn::http1;
use hyper_util::{rt::TokioIo, service::TowerToHyperService};
use rcgen::generate_simple_self_signed;
use serde::Deserialize;
use tokio::{net::TcpListener, sync::watch, task::JoinSet};
use tokio_rustls::{
    TlsAcceptor,
    rustls::{
        ServerConfig as RustlsServerConfig,
        pki_types::{CertificateDer, PrivatePkcs8KeyDer},
    },
};
use tokio_stream::wrappers::BroadcastStream;
use tracing::info;
use uuid::Uuid;

#[cfg(test)]
use crate::sessions::TurnHandle;
use crate::{
    auth::{AuthError, CSRF_COOKIE_NAME, SESSION_COOKIE_NAME, authorize_request, cookie_value},
    completions::resolve_slash_completions,
    mock_client::{MockClient, MockClientError, ReplyProvider, ReplyResult},
    sessions::{PendingPrompt, SessionStore, SessionStoreError},
};

const ACCEPT_ERROR_BACKOFF: Duration = Duration::from_millis(50);
const CONNECTION_SHUTDOWN_GRACE_PERIOD: Duration = Duration::from_millis(500);
const MAX_CONSECUTIVE_TRANSIENT_ACCEPT_ERRORS: usize = 50;
const SHUTDOWN_DRAIN_GRACE_PERIOD: Duration = Duration::from_secs(1);

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub session_cap: usize,
    pub acp_server: String,
    pub startup_hints: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            session_cap: 8,
            acp_server: "127.0.0.1:8090".to_string(),
            startup_hints: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AppState {
    store: Arc<SessionStore>,
    reply_provider: Arc<dyn ReplyProvider>,
    startup_hints: bool,
}

impl AppState {
    pub fn new(config: ServerConfig) -> Result<Self, MockClientError> {
        Ok(Self {
            store: Arc::new(SessionStore::new(config.session_cap)),
            reply_provider: Arc::new(MockClient::new(config.acp_server)?),
            startup_hints: config.startup_hints,
        })
    }

    pub fn with_dependencies(
        store: Arc<SessionStore>,
        reply_provider: Arc<dyn ReplyProvider>,
    ) -> Self {
        Self {
            store,
            reply_provider,
            startup_hints: false,
        }
    }
}

pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/app", get(redirect_to_app))
        .route("/app/", get(app_entrypoint))
        .route("/app/sessions/{session_id}", get(app_session_entrypoint))
        .route("/api/v1/sessions", get(list_sessions).post(create_session))
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
        .route("/api/v1/sessions/{session_id}/cancel", post(cancel_turn))
        .route(
            "/api/v1/sessions/{session_id}/permissions/{request_id}",
            post(resolve_permission),
        )
        .route("/api/v1/sessions/{session_id}/close", post(close_session))
        .route("/api/v1/completions/slash", get(get_slash_completions))
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
    let tls_acceptor = build_loopback_tls_acceptor(address)?;
    let app = app(state);
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let mut connections = JoinSet::new();
    let mut consecutive_transient_accept_errors = 0usize;
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            _ = &mut shutdown => break,
            next = connections.join_next(), if !connections.is_empty() => {
                if let Some(Err(error)) = next {
                    tracing::warn!(%error, "web backend connection task aborted");
                }
            }
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, _)) => {
                        consecutive_transient_accept_errors = 0;
                        let acceptor = tls_acceptor.clone();
                        let service = TowerToHyperService::new(app.clone());
                        let mut connection_shutdown = shutdown_rx.clone();
                        connections.spawn(async move {
                            let tls_stream = match acceptor.accept(stream).await {
                                Ok(stream) => stream,
                                Err(error) => {
                                    tracing::warn!(%error, "failed to complete the loopback TLS handshake");
                                    return;
                                }
                            };
                            let io = TokioIo::new(tls_stream);
                            let connection = http1::Builder::new().serve_connection(io, service);
                            tokio::pin!(connection);

                            tokio::select! {
                                result = &mut connection => {
                                    if let Err(error) = result {
                                        tracing::warn!(%error, "web backend connection failed");
                                    }
                                }
                                changed = connection_shutdown.changed() => {
                                    if changed.is_ok() && *connection_shutdown.borrow() {
                                        connection.as_mut().graceful_shutdown();
                                        match tokio::time::timeout(
                                            CONNECTION_SHUTDOWN_GRACE_PERIOD,
                                            connection.as_mut(),
                                        )
                                        .await
                                        {
                                            Ok(Ok(())) => {}
                                            Ok(Err(error)) => {
                                                tracing::warn!(%error, "web backend connection failed during graceful shutdown");
                                            }
                                            Err(_) => {
                                                tracing::warn!("web backend connection exceeded the graceful shutdown deadline");
                                            }
                                        }
                                    }
                                }
                            }
                        });
                    }
                    Err(error) if accept_error_is_transient(&error) => {
                        consecutive_transient_accept_errors += 1;
                        if consecutive_transient_accept_errors > MAX_CONSECUTIVE_TRANSIENT_ACCEPT_ERRORS {
                            tracing::error!(
                                %error,
                                failures = consecutive_transient_accept_errors,
                                "too many transient accept failures while serving the web backend"
                            );
                            shutdown_connections(&shutdown_tx, &mut connections).await;
                            return Err(error);
                        }
                        tracing::warn!(
                            %error,
                            failures = consecutive_transient_accept_errors,
                            "transient accept failure while serving the web backend"
                        );
                        tokio::select! {
                            _ = &mut shutdown => break,
                            _ = tokio::time::sleep(ACCEPT_ERROR_BACKOFF) => {}
                        }
                    }
                    Err(error) => {
                        shutdown_connections(&shutdown_tx, &mut connections).await;
                        return Err(error);
                    }
                }
            }
        }
    }

    shutdown_connections(&shutdown_tx, &mut connections).await;
    Ok(())
}

async fn shutdown_connections(shutdown_tx: &watch::Sender<bool>, connections: &mut JoinSet<()>) {
    let _ = shutdown_tx.send(true);
    drain_connection_tasks(connections).await;
}

async fn drain_connection_tasks(connections: &mut JoinSet<()>) {
    let shutdown_deadline = tokio::time::sleep(SHUTDOWN_DRAIN_GRACE_PERIOD);
    tokio::pin!(shutdown_deadline);
    loop {
        tokio::select! {
            _ = &mut shutdown_deadline, if !connections.is_empty() => {
                connections.abort_all();
                while connections.join_next().await.is_some() {}
                return;
            }
            next = connections.join_next(), if !connections.is_empty() => {
                if let Some(Err(error)) = next {
                    tracing::warn!(%error, "web backend connection task aborted");
                }
            }
            else => return,
        }
    }
}

async fn healthz() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
    })
}

async fn redirect_to_app() -> Redirect {
    Redirect::permanent("/app/")
}

async fn app_entrypoint(headers: HeaderMap) -> Response {
    app_shell_response(&headers)
}

async fn app_session_entrypoint(Path(_session_id): Path<String>, headers: HeaderMap) -> Response {
    app_shell_response(&headers)
}

fn app_shell_response(headers: &HeaderMap) -> Response {
    let existing_session_id = cookie_uuid_value(headers, SESSION_COOKIE_NAME);
    let session_id = existing_session_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let existing_csrf_token = cookie_uuid_value(headers, CSRF_COOKIE_NAME);
    let csrf_token = existing_csrf_token
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let mut response_headers = HeaderMap::new();
    response_headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    response_headers.insert(
        "content-security-policy",
        HeaderValue::from_static(
            "default-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; connect-src 'self'",
        ),
    );
    response_headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    response_headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response_headers.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));

    if existing_session_id.is_none() {
        response_headers.append(
            SET_COOKIE,
            build_cookie_header(SESSION_COOKIE_NAME, &session_id, true),
        );
    }
    if existing_csrf_token.is_none() {
        response_headers.append(
            SET_COOKIE,
            build_cookie_header(CSRF_COOKIE_NAME, &csrf_token, false),
        );
    }

    (
        response_headers,
        Html(format!(
            r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <meta name="acp-api-base" content="/api/v1">
    <meta name="acp-csrf-token" content="{csrf_token}">
    <title>ACP Web MVP slice 0</title>
  </head>
  <body>
    <main>
      <h1>ACP Web MVP slice 0</h1>
      <p>Loopback HTTPS browser entrypoint is ready.</p>
      <p>This slice fixes the launcher, auth cookie bootstrap, and CSRF bootstrap before the chat UI arrives.</p>
      <p>Route vocabulary is fixed at <code>/app/</code> and <code>/app/sessions/{{id}}</code>.</p>
    </main>
  </body>
</html>
"#
        )),
    )
        .into_response()
}

fn build_cookie_header(name: &str, value: &str, http_only: bool) -> HeaderValue {
    let http_only = if http_only { "; HttpOnly" } else { "" };
    assert!(
        name.bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')),
        "slice 0 cookie names must stay header-safe",
    );
    assert!(
        value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'),
        "slice 0 cookie values must stay UUID-safe",
    );
    HeaderValue::from_str(&format!(
        "{name}={value}; Path=/; SameSite=Strict; Secure{http_only}"
    ))
    .expect("slice 0 cookies should serialize into response headers")
}

fn cookie_uuid_value(headers: &HeaderMap, name: &str) -> Option<String> {
    cookie_value(headers, name).and_then(|value| {
        Uuid::parse_str(value)
            .ok()
            .map(|uuid| uuid.as_hyphenated().to_string())
    })
}

fn build_loopback_tls_acceptor(address: SocketAddr) -> io::Result<TlsAcceptor> {
    let mut subject_alt_names = vec![
        "localhost".to_string(),
        "127.0.0.1".to_string(),
        "::1".to_string(),
    ];
    let bound_host = address.ip().to_string();
    if !subject_alt_names.iter().any(|name| name == &bound_host) {
        subject_alt_names.push(bound_host);
    }

    let certified = generate_simple_self_signed(subject_alt_names).map_err(io::Error::other)?;
    let mut config = RustlsServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(certified.cert.der().to_vec())],
            PrivatePkcs8KeyDer::from(certified.signing_key.serialize_der()).into(),
        )
        .map_err(io::Error::other)?;
    config.alpn_protocols = vec![b"http/1.1".to_vec()];

    Ok(TlsAcceptor::from(Arc::new(config)))
}

fn accept_error_is_transient(error: &io::Error) -> bool {
    if matches!(
        error.kind(),
        io::ErrorKind::ConnectionAborted
            | io::ErrorKind::Interrupted
            | io::ErrorKind::TimedOut
            | io::ErrorKind::WouldBlock
    ) {
        return true;
    }

    #[cfg(unix)]
    {
        matches!(
            error.raw_os_error(),
            Some(
                libc::ECONNABORTED
                    | libc::EINTR
                    | libc::EMFILE
                    | libc::ENFILE
                    | libc::ENOBUFS
                    | libc::ENOMEM
            )
        )
    }

    #[cfg(not(unix))]
    {
        false
    }
}

async fn list_sessions(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<SessionListResponse>, AppError> {
    let principal = authorize_request(&headers, false)?;
    let sessions = state.store.list_owned_sessions(&principal.id).await;

    Ok(Json(SessionListResponse { sessions }))
}

async fn create_session(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<(StatusCode, Json<CreateSessionResponse>), AppError> {
    let principal = authorize_request(&headers, true)?;
    let session = state.store.create_session(&principal.id).await?;
    let session_id = session.id.clone();
    let session = match seed_startup_hint(&state, &principal.id, session).await {
        Ok(session) => session,
        Err(error) => {
            if let Err(rollback_error) =
                rollback_failed_session(&state, &principal.id, &session_id).await
            {
                return Err(AppError::Internal(format!(
                    "{}; session rollback failed: {}",
                    error.message(),
                    rollback_error.message()
                )));
            }
            return Err(error);
        }
    };

    Ok((StatusCode::CREATED, Json(CreateSessionResponse { session })))
}

async fn seed_startup_hint(
    state: &AppState,
    owner: &str,
    session: SessionSnapshot,
) -> Result<SessionSnapshot, AppError> {
    if !state.startup_hints {
        return Ok(session);
    }

    let Some(hint) = state
        .reply_provider
        .prime_session(&session.id)
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?
    else {
        return Ok(session);
    };

    state
        .store
        .append_assistant_message(owner, &session.id, hint)
        .await
        .map_err(AppError::from)
}

async fn rollback_failed_session(
    state: &AppState,
    owner: &str,
    session_id: &str,
) -> Result<(), AppError> {
    state.reply_provider.forget_session(session_id);
    state
        .store
        .discard_session(owner, session_id)
        .await
        .map_err(|error| AppError::Internal(error.message().to_string()))
}

async fn get_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<CreateSessionResponse>, AppError> {
    let principal = authorize_request(&headers, false)?;
    let session = state.store.open_session(&principal.id, &session_id).await?;

    Ok(Json(CreateSessionResponse { session }))
}

async fn get_session_history(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<SessionHistoryResponse>, AppError> {
    let principal = authorize_request(&headers, false)?;
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
    let principal = authorize_request(&headers, true)?;
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
    let principal = authorize_request(&headers, true)?;
    let session = state
        .store
        .close_session(&principal.id, &session_id)
        .await?;
    state.reply_provider.forget_session(&session_id);

    Ok(Json(CloseSessionResponse { session }))
}

async fn cancel_turn(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<CancelTurnResponse>, AppError> {
    let principal = authorize_request(&headers, true)?;
    let cancelled = state
        .store
        .cancel_active_turn(&principal.id, &session_id)
        .await?;

    Ok(Json(CancelTurnResponse { cancelled }))
}

async fn resolve_permission(
    State(state): State<AppState>,
    Path((session_id, request_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(request): Json<ResolvePermissionRequest>,
) -> Result<Json<ResolvePermissionResponse>, AppError> {
    let principal = authorize_request(&headers, true)?;
    let resolution = state
        .store
        .resolve_permission(&principal.id, &session_id, &request_id, request.decision)
        .await?;

    Ok(Json(resolution))
}

#[derive(Debug, Deserialize)]
struct SlashCompletionsQuery {
    #[serde(rename = "sessionId")]
    session_id: String,
    #[serde(default)]
    prefix: String,
}

async fn get_slash_completions(
    State(state): State<AppState>,
    Query(query): Query<SlashCompletionsQuery>,
    headers: HeaderMap,
) -> Result<Json<SlashCompletionsResponse>, AppError> {
    let principal = authorize_request(&headers, false)?;
    let response_future = resolve_slash_completions(
        &state.store,
        &principal.id,
        &query.session_id,
        &query.prefix,
    );
    let response = response_future.await?;

    Ok(Json(response))
}

async fn stream_session_events(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let principal = authorize_request(&headers, false)?;
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
    tokio::spawn(async move {
        match reply_provider.request_reply(pending.turn_handle()).await {
            Ok(ReplyResult::Reply(reply)) => pending.complete_with_reply(reply).await,
            Ok(ReplyResult::Status(message)) => pending.complete_with_status(message).await,
            Ok(ReplyResult::NoOutput) => pending.complete_without_output().await,
            Err(error) => {
                pending
                    .complete_with_status(format!("ACP request failed: {error}"))
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
            AuthError::MissingAuthentication | AuthError::InvalidAuthentication => {
                Self::Unauthorized(error.message().to_string())
            }
            AuthError::MissingCsrfToken | AuthError::InvalidCsrfToken => {
                Self::Forbidden(error.message().to_string())
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
            SessionStoreError::PermissionNotFound => Self::NotFound(error.message().to_string()),
            SessionStoreError::SessionCapReached => {
                Self::TooManyRequests(error.message().to_string())
            }
        }
    }
}

#[cfg(test)]
mod tests;
