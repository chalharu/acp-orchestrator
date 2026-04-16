use std::{
    convert::Infallible, fmt::Display, future::Future, io, net::SocketAddr, pin::Pin, sync::Arc,
    time::Duration,
};

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
        .route("/app/assets/app.css", get(app_stylesheet))
        .route("/app/assets/app.js", get(app_javascript))
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
                log_connection_task_join_result(next);
            }
            accepted = listener.accept() => {
                let should_break = matches!(
                    handle_accept_result(
                    accepted,
                    &mut consecutive_transient_accept_errors,
                    AcceptContext {
                        connections: &mut connections,
                        tls_acceptor: &tls_acceptor,
                        app: &app,
                        shutdown_rx: &shutdown_rx,
                        shutdown_tx: &shutdown_tx,
                    },
                    shutdown.as_mut(),
                )
                    .await?,
                    AcceptLoopAction::Break
                );
                if should_break { break; }
            }
        }
    }

    shutdown_connections(&shutdown_tx, &mut connections).await;
    Ok(())
}

fn log_connection_task_join_result(next: Option<Result<(), tokio::task::JoinError>>) {
    if let Some(Err(error)) = next {
        tracing::warn!(%error, "web backend connection task aborted");
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AcceptLoopAction {
    Continue,
    Break,
}

struct AcceptContext<'a> {
    connections: &'a mut JoinSet<()>,
    tls_acceptor: &'a TlsAcceptor,
    app: &'a Router,
    shutdown_rx: &'a watch::Receiver<bool>,
    shutdown_tx: &'a watch::Sender<bool>,
}

async fn handle_accept_result<F>(
    accepted: io::Result<(tokio::net::TcpStream, SocketAddr)>,
    consecutive_transient_accept_errors: &mut usize,
    context: AcceptContext<'_>,
    shutdown: Pin<&mut F>,
) -> io::Result<AcceptLoopAction>
where
    F: Future<Output = ()>,
{
    match accepted {
        Ok((stream, _)) => {
            *consecutive_transient_accept_errors = 0;
            spawn_connection_task(
                context.connections,
                context.tls_acceptor.clone(),
                context.app.clone(),
                context.shutdown_rx.clone(),
                stream,
            );
            Ok(AcceptLoopAction::Continue)
        }
        Err(error) if accept_error_is_transient(&error) => {
            *consecutive_transient_accept_errors += 1;
            if *consecutive_transient_accept_errors > MAX_CONSECUTIVE_TRANSIENT_ACCEPT_ERRORS {
                tracing::error!(
                    %error,
                    failures = *consecutive_transient_accept_errors,
                    "too many transient accept failures while serving the web backend"
                );
                shutdown_connections(context.shutdown_tx, context.connections).await;
                return Err(error);
            }
            tracing::warn!(
                %error,
                failures = *consecutive_transient_accept_errors,
                "transient accept failure while serving the web backend"
            );
            Ok(wait_for_accept_retry_or_shutdown(shutdown).await)
        }
        Err(error) => {
            shutdown_connections(context.shutdown_tx, context.connections).await;
            Err(error)
        }
    }
}

async fn wait_for_accept_retry_or_shutdown<F>(shutdown: Pin<&mut F>) -> AcceptLoopAction
where
    F: Future<Output = ()>,
{
    tokio::select! {
        _ = shutdown => AcceptLoopAction::Break,
        _ = tokio::time::sleep(ACCEPT_ERROR_BACKOFF) => AcceptLoopAction::Continue,
    }
}

fn log_connection_result<E: Display>(result: Result<(), E>) {
    if let Err(error) = result {
        tracing::warn!(%error, "web backend connection failed");
    }
}

fn spawn_connection_task(
    connections: &mut JoinSet<()>,
    acceptor: TlsAcceptor,
    app: Router,
    mut connection_shutdown: watch::Receiver<bool>,
    stream: tokio::net::TcpStream,
) {
    connections.spawn(async move {
        let tls_stream = match acceptor.accept(stream).await {
            Ok(stream) => stream,
            Err(error) => {
                tracing::warn!(%error, "failed to complete the loopback TLS handshake");
                return;
            }
        };
        let io = TokioIo::new(tls_stream);
        let connection = http1::Builder::new().serve_connection(io, TowerToHyperService::new(app));
        tokio::pin!(connection);

        #[rustfmt::skip]
        tokio::select! {
            result = &mut connection => log_connection_result(result),
            changed = connection_shutdown.changed() => {
                if changed.is_ok() && *connection_shutdown.borrow() { connection.as_mut().graceful_shutdown(); finish_connection_after_shutdown(connection.as_mut()).await; }
            }
        }
    });
}

async fn finish_connection_after_shutdown<F, E>(connection: Pin<&mut F>)
where
    F: Future<Output = Result<(), E>>,
    E: Display,
{
    match tokio::time::timeout(CONNECTION_SHUTDOWN_GRACE_PERIOD, connection).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            tracing::warn!(%error, "web backend connection failed during graceful shutdown");
        }
        Err(_) => {
            tracing::warn!("web backend connection exceeded the graceful shutdown deadline");
        }
    }
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
                log_connection_task_join_result(next);
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

async fn app_stylesheet() -> Response {
    app_asset_response("text/css; charset=utf-8", APP_STYLESHEET)
}

async fn app_javascript() -> Response {
    app_asset_response("application/javascript; charset=utf-8", APP_JAVASCRIPT)
}

fn app_shell_response(headers: &HeaderMap) -> Response {
    let (existing_session_id, session_id) = app_shell_cookie(headers, SESSION_COOKIE_NAME);
    let (existing_csrf_token, csrf_token) = app_shell_cookie(headers, CSRF_COOKIE_NAME);

    (
        build_app_shell_headers(
            existing_session_id.as_deref(),
            &session_id,
            existing_csrf_token.as_deref(),
            &csrf_token,
        ),
        app_shell_document(&csrf_token),
    )
        .into_response()
}

fn app_asset_response(content_type: &'static str, body: &'static str) -> Response {
    let mut response_headers = HeaderMap::new();
    response_headers.insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
    response_headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response_headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    response_headers.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));

    (response_headers, body).into_response()
}

fn app_shell_cookie(headers: &HeaderMap, name: &str) -> (Option<String>, String) {
    let existing = cookie_uuid_value(headers, name);
    let value = existing
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    (existing, value)
}

fn build_app_shell_headers(
    existing_session_id: Option<&str>,
    session_id: &str,
    existing_csrf_token: Option<&str>,
    csrf_token: &str,
) -> HeaderMap {
    let mut response_headers = HeaderMap::new();
    response_headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    response_headers.insert(
        "content-security-policy",
        HeaderValue::from_static(
            "default-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'; img-src 'self' data:; style-src 'self'; script-src 'self'; connect-src 'self'",
        ),
    );
    response_headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    response_headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response_headers.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    append_cookie_if_missing(
        &mut response_headers,
        existing_session_id,
        SESSION_COOKIE_NAME,
        session_id,
        true,
    );
    append_cookie_if_missing(
        &mut response_headers,
        existing_csrf_token,
        CSRF_COOKIE_NAME,
        csrf_token,
        false,
    );
    response_headers
}

fn append_cookie_if_missing(
    headers: &mut HeaderMap,
    existing: Option<&str>,
    name: &str,
    value: &str,
    http_only: bool,
) {
    if existing.is_none() {
        headers.append(SET_COOKIE, build_cookie_header(name, value, http_only));
    }
}

fn app_shell_document(csrf_token: &str) -> Html<String> {
    Html(format!(
        r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <meta name="acp-api-base" content="/api/v1">
    <meta name="acp-csrf-token" content="{csrf_token}">
    <title>ACP Web chat</title>
    <link rel="stylesheet" href="/app/assets/app.css">
  </head>
  <body>
    <main class="app-shell">
      <header class="app-header">
        <div class="app-header__copy">
          <p class="eyebrow">ACP Web MVP slice 1</p>
          <h1>ACP Web chat</h1>
          <p id="route-summary" class="muted">Send the first prompt to create a session and move to <code>/app/sessions/{{id}}</code>.</p>
        </div>
        <dl class="status-grid" aria-label="session status">
          <div>
            <dt>Backend</dt>
            <dd id="backend-origin"></dd>
          </div>
          <div>
            <dt>Connection</dt>
            <dd id="connection-status">bootstrapping</dd>
          </div>
          <div>
            <dt>Session</dt>
            <dd id="session-status">new</dd>
          </div>
        </dl>
      </header>

      <p class="top-link"><a href="/app/">Start a fresh chat</a></p>

      <div id="error-banner" class="banner" hidden role="alert"></div>

      <section class="panel transcript-panel" aria-label="conversation transcript">
        <ol id="transcript" class="transcript"></ol>
      </section>

      <section id="pending-permissions-panel" class="panel pending-panel" hidden aria-live="polite">
        <h2>Pending permissions</h2>
        <p class="muted">Slice 1 renders pending requests and transcript updates. Approve/deny controls arrive in the next slice.</p>
        <ul id="pending-permissions" class="pending-list"></ul>
      </section>

      <form id="composer-form" class="panel composer" autocomplete="off">
        <label for="composer-input">Prompt</label>
        <textarea id="composer-input" name="prompt" rows="4" placeholder="Ask ACP something…"></textarea>
        <div class="composer__footer">
          <p id="composer-status" class="muted">Ready for your first prompt.</p>
          <button id="composer-submit" type="submit">Send</button>
        </div>
      </form>
    </main>
    <script src="/app/assets/app.js"></script>
  </body>
</html>
"#
    ))
}

const APP_STYLESHEET: &str = include_str!("app_assets/app.css");
const APP_JAVASCRIPT: &str = include_str!("app_assets/app.js");

fn build_cookie_header(name: &str, value: &str, http_only: bool) -> HeaderValue {
    let http_only = if http_only { "; HttpOnly" } else { "" };
    assert!(
        name.bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')),
        "web app cookie names must stay header-safe",
    );
    assert!(
        value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'),
        "web app cookie values must stay UUID-safe",
    );
    HeaderValue::from_str(&format!(
        "{name}={value}; Path=/; SameSite=Strict; Secure{http_only}"
    ))
    .expect("web app cookies should serialize into response headers")
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
