use std::{
    convert::Infallible,
    fmt::Display,
    future::Future,
    io,
    net::SocketAddr,
    path::{Path as FsPath, PathBuf},
    pin::Pin,
    sync::Arc,
    time::Duration,
};

use acp_app_support::{
    FRONTEND_JAVASCRIPT_ASSET_PATH, FRONTEND_WASM_ASSET_PATH, FrontendBundleAsset,
    LEGACY_FRONTEND_JAVASCRIPT_ASSET_PATH, LEGACY_FRONTEND_WASM_ASSET_PATH,
    find_frontend_bundle_asset,
};
use acp_contracts::{
    AccountListResponse, AuthStatusResponse, BootstrapRegistrationRequest,
    BootstrapRegistrationResponse, CancelTurnResponse, CloseSessionResponse, CreateAccountRequest,
    CreateAccountResponse, CreateSessionResponse, DeleteAccountResponse, DeleteSessionResponse,
    ErrorResponse, HealthResponse, LocalAccount, PromptRequest, PromptResponse,
    RenameSessionRequest, RenameSessionResponse, ResolvePermissionRequest,
    ResolvePermissionResponse, SessionHistoryResponse, SessionListResponse, SessionResponse,
    SessionSnapshot, SignInRequest, SignInResponse, SlashCompletionsResponse, StreamEvent,
    UpdateAccountRequest, UpdateAccountResponse,
};
use axum::{
    Json, Router,
    extract::{Extension, Path, Query, Request, State},
    http::{
        HeaderMap, HeaderValue, StatusCode,
        header::{CACHE_CONTROL, CONTENT_TYPE, REFERRER_POLICY, SET_COOKIE},
    },
    middleware::{self, Next},
    response::{
        Html, IntoResponse, Redirect, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, patch, post},
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
#[cfg(test)]
use crate::workspace_store::SqliteWorkspaceRepository;
use crate::{
    auth::{
        AuthError, AuthenticatedPrincipal, CSRF_COOKIE_NAME, SESSION_COOKIE_NAME,
        authorize_request, cookie_value,
    },
    completions::resolve_slash_completions,
    mock_client::{MockClient, MockClientError, ReplyProvider, ReplyResult},
    sessions::{PendingPrompt, SessionStore, SessionStoreError},
    workspace_repository::WorkspaceRepository,
    workspace_store::{UserRecord, WorkspaceStoreError},
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
    pub state_dir: PathBuf,
    /// Directory containing the Trunk-compiled Leptos CSR bundle.
    /// The backend serves the fingerprinted files through stable alias routes.
    /// When `None`, the WASM asset routes return `503 Service Unavailable`.
    pub frontend_dist: Option<PathBuf>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            session_cap: 8,
            acp_server: "127.0.0.1:8090".to_string(),
            startup_hints: false,
            state_dir: PathBuf::from(".acp-state"),
            frontend_dist: None,
        }
    }
}

#[derive(Debug)]
pub enum AppStateBuildError {
    ReplyProvider(MockClientError),
    WorkspaceStore(WorkspaceStoreError),
}

impl AppStateBuildError {
    fn message(&self) -> String {
        match self {
            Self::ReplyProvider(source) => source.to_string(),
            Self::WorkspaceStore(source) => source.to_string(),
        }
    }
}

impl Display for AppStateBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

impl std::error::Error for AppStateBuildError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::ReplyProvider(source) => Some(source),
            Self::WorkspaceStore(source) => Some(source),
        }
    }
}

impl From<MockClientError> for AppStateBuildError {
    fn from(source: MockClientError) -> Self {
        Self::ReplyProvider(source)
    }
}

impl From<WorkspaceStoreError> for AppStateBuildError {
    fn from(source: WorkspaceStoreError) -> Self {
        Self::WorkspaceStore(source)
    }
}

#[derive(Clone)]
pub struct AppState {
    store: Arc<SessionStore>,
    workspace_repository: Arc<dyn WorkspaceRepository>,
    reply_provider: Arc<dyn ReplyProvider>,
    startup_hints: bool,
    /// Path to the Trunk dist directory.  `None` → WASM routes return 503.
    frontend_dist: Option<Arc<PathBuf>>,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("store", &self.store)
            .field("startup_hints", &self.startup_hints)
            .field("frontend_dist", &self.frontend_dist)
            .finish()
    }
}

impl AppState {
    pub fn new(
        config: ServerConfig,
        workspace_repository: Arc<dyn WorkspaceRepository>,
    ) -> Result<Self, AppStateBuildError> {
        Ok(Self {
            store: Arc::new(SessionStore::new(config.session_cap)),
            workspace_repository,
            reply_provider: Arc::new(MockClient::new(config.acp_server)?),
            startup_hints: config.startup_hints,
            frontend_dist: config.frontend_dist.map(Arc::new),
        })
    }

    #[cfg(test)]
    pub fn with_dependencies(
        store: Arc<SessionStore>,
        reply_provider: Arc<dyn ReplyProvider>,
    ) -> Self {
        Self {
            store,
            workspace_repository: new_ephemeral_workspace_repository(),
            reply_provider,
            startup_hints: false,
            frontend_dist: None,
        }
    }

    #[cfg(test)]
    pub fn with_workspace_repository(
        store: Arc<SessionStore>,
        workspace_repository: Arc<dyn WorkspaceRepository>,
        reply_provider: Arc<dyn ReplyProvider>,
    ) -> Self {
        Self {
            store,
            workspace_repository,
            reply_provider,
            startup_hints: false,
            frontend_dist: None,
        }
    }

    async fn owner_context(
        &self,
        principal: AuthenticatedPrincipal,
    ) -> Result<OwnerContext, AppError> {
        let user = match principal.kind {
            crate::auth::AuthenticatedPrincipalKind::Bearer => {
                self.workspace_repository
                    .materialize_user(&principal)
                    .await?
            }
            crate::auth::AuthenticatedPrincipalKind::BrowserSession => self
                .workspace_repository
                .authenticate_browser_session(&principal.id)
                .await?
                .ok_or_else(|| {
                    AppError::Unauthorized("account authentication required".to_string())
                })?,
        };
        Ok(OwnerContext { principal, user })
    }
}

pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/app", get(redirect_to_app))
        .route("/app/", get(app_entrypoint))
        .route("/app/register", get(redirect_to_register))
        .route("/app/register/", get(app_register_entrypoint))
        .route("/app/sign-in", get(redirect_to_sign_in))
        .route("/app/sign-in/", get(app_sign_in_entrypoint))
        .route("/app/accounts", get(redirect_to_accounts))
        .route("/app/accounts/", get(app_accounts_entrypoint))
        .route("/app/assets/app.css", get(app_stylesheet))
        .route("/app/assets/fonts/{font_name}", get(app_font_asset))
        .route("/app/assets/wasm-init.js", get(wasm_init_script))
        .route(FRONTEND_JAVASCRIPT_ASSET_PATH, get(wasm_glue_javascript))
        .route(FRONTEND_WASM_ASSET_PATH, get(wasm_binary))
        .route(
            LEGACY_FRONTEND_JAVASCRIPT_ASSET_PATH,
            get(wasm_glue_javascript),
        )
        .route(LEGACY_FRONTEND_WASM_ASSET_PATH, get(wasm_binary))
        .route("/app/sessions/{session_id}", get(app_session_entrypoint))
        .route("/api/v1/auth/status", get(auth_status))
        .merge(read_api_routes())
        .merge(write_api_routes())
        .with_state(state)
}

#[derive(Debug, Clone)]
struct OwnerContext {
    principal: AuthenticatedPrincipal,
    user: UserRecord,
}

#[derive(Debug, Clone)]
struct LiveSessionWriteContext {
    principal: AuthenticatedPrincipal,
    user: Option<UserRecord>,
}

fn read_api_routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/accounts", get(list_accounts))
        .route("/api/v1/sessions", get(list_sessions))
        .route("/api/v1/sessions/{session_id}", get(get_session))
        .route(
            "/api/v1/sessions/{session_id}/history",
            get(get_session_history),
        )
        .route(
            "/api/v1/sessions/{session_id}/events",
            get(stream_session_events),
        )
        .route("/api/v1/completions/slash", get(get_slash_completions))
        .layer(middleware::from_fn(authorize_read_request))
}

fn write_api_routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/auth/sign-in", post(sign_in))
        .route("/api/v1/bootstrap/register", post(bootstrap_register))
        .route("/api/v1/sessions", post(create_session))
        .route("/api/v1/accounts", post(create_account))
        .route(
            "/api/v1/accounts/{user_id}",
            patch(update_account).delete(delete_account),
        )
        .route(
            "/api/v1/sessions/{session_id}",
            patch(rename_session).delete(delete_session),
        )
        .route("/api/v1/sessions/{session_id}/messages", post(post_message))
        .route("/api/v1/sessions/{session_id}/cancel", post(cancel_turn))
        .route(
            "/api/v1/sessions/{session_id}/permissions/{request_id}",
            post(resolve_permission),
        )
        .route("/api/v1/sessions/{session_id}/close", post(close_session))
        .layer(middleware::from_fn(authorize_write_request))
}

async fn authorize_read_request(request: Request, next: Next) -> Result<Response, AppError> {
    authorize_request_with_principal(request, next, false).await
}

async fn authorize_write_request(request: Request, next: Next) -> Result<Response, AppError> {
    authorize_request_with_principal(request, next, true).await
}

async fn authorize_request_with_principal(
    mut request: Request,
    next: Next,
    requires_csrf: bool,
) -> Result<Response, AppError> {
    let principal = authorize_request(request.headers(), requires_csrf).map_err(AppError::from)?;
    request.extensions_mut().insert(principal);
    Ok(next.run(request).await)
}

#[cfg(test)]
fn new_ephemeral_workspace_repository() -> Arc<dyn WorkspaceRepository> {
    let db_path = std::env::temp_dir().join(format!(
        "acp-web-backend-test-state-{}",
        Uuid::new_v4().simple()
    ));
    Arc::new(
        SqliteWorkspaceRepository::new(db_path.join("db.sqlite"))
            .expect("ephemeral workspace repository should initialize"),
    )
}

async fn persist_session_metadata(
    state: &AppState,
    user: &UserRecord,
    snapshot: &SessionSnapshot,
    touch_activity: bool,
    status_override: Option<&str>,
) -> Result<(), AppError> {
    state
        .workspace_repository
        .persist_session_snapshot(&user.user_id, snapshot, touch_activity, status_override)
        .await
        .map_err(AppError::from)?;
    Ok(())
}

async fn persist_session_metadata_best_effort(
    state: &AppState,
    user: &UserRecord,
    snapshot: &SessionSnapshot,
    touch_activity: bool,
    status_override: Option<&str>,
    action: &'static str,
) {
    if let Err(error) =
        persist_session_metadata(state, user, snapshot, touch_activity, status_override).await
    {
        let error_message = error.message();
        tracing::warn!(
            session_id = %snapshot.id,
            owner_user_id = %user.user_id,
            action,
            "failed to persist session metadata: {error_message}"
        );
    }
}

async fn persist_session_metadata_for_user_best_effort(
    state: &AppState,
    user: &UserRecord,
    snapshot: &SessionSnapshot,
    touch_activity: bool,
    status_override: Option<&str>,
    action: &'static str,
) {
    persist_session_metadata_best_effort(
        state,
        user,
        snapshot,
        touch_activity,
        status_override,
        action,
    )
    .await;
}

async fn materialize_user_best_effort(
    state: &AppState,
    principal: &AuthenticatedPrincipal,
    action: &'static str,
) -> Option<UserRecord> {
    match state.workspace_repository.materialize_user(principal).await {
        Ok(user) => Some(user),
        Err(error) => {
            let error = AppError::from(error);
            let error_message = error.message();
            tracing::warn!(
                %error_message,
                principal_kind = ?principal.kind,
                action,
                "failed to materialize durable user"
            );
            None
        }
    }
}

async fn live_session_write_context(
    state: &AppState,
    principal: AuthenticatedPrincipal,
    action: &'static str,
) -> Result<LiveSessionWriteContext, AppError> {
    match principal.kind {
        crate::auth::AuthenticatedPrincipalKind::Bearer => Ok(LiveSessionWriteContext {
            user: materialize_user_best_effort(state, &principal, action).await,
            principal,
        }),
        crate::auth::AuthenticatedPrincipalKind::BrowserSession => {
            let owner = state.owner_context(principal).await?;
            Ok(LiveSessionWriteContext {
                principal: owner.principal,
                user: Some(owner.user),
            })
        }
    }
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

async fn persist_prompt_snapshot_best_effort(
    state: &AppState,
    user: &UserRecord,
    session_id: &str,
    snapshot_result: Result<SessionSnapshot, SessionStoreError>,
) {
    match snapshot_result {
        Ok(snapshot) => {
            persist_session_metadata_for_user_best_effort(
                state,
                user,
                &snapshot,
                true,
                None,
                "submit_prompt",
            )
            .await;
        }
        Err(error) => {
            let error_message = error.message();
            tracing::warn!(
                session_id = %session_id,
                "failed to snapshot session metadata after prompt submission: {error_message}"
            );
        }
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

async fn redirect_to_register() -> Redirect {
    Redirect::permanent("/app/register/")
}

async fn redirect_to_sign_in() -> Redirect {
    Redirect::permanent("/app/sign-in/")
}

async fn redirect_to_accounts() -> Redirect {
    Redirect::permanent("/app/accounts/")
}

async fn app_entrypoint(headers: HeaderMap) -> Response {
    app_shell_response(&headers)
}

async fn app_register_entrypoint(headers: HeaderMap) -> Response {
    app_shell_response(&headers)
}

async fn app_sign_in_entrypoint(headers: HeaderMap) -> Response {
    app_shell_response(&headers)
}

async fn app_accounts_entrypoint(headers: HeaderMap) -> Response {
    app_shell_response(&headers)
}

async fn app_session_entrypoint(Path(_session_id): Path<String>, headers: HeaderMap) -> Response {
    app_shell_response(&headers)
}

async fn app_stylesheet() -> Response {
    app_static_text_response("text/css; charset=utf-8", APP_STYLESHEET)
}

async fn app_font_asset(Path(font_name): Path<String>) -> Response {
    match font_name.as_str() {
        "noto-sans-jp-latin-400.woff2" => {
            app_static_font_response(APP_FONT_NOTO_SANS_JP_LATIN_REGULAR)
        }
        "noto-sans-jp-japanese-400.woff2" => {
            app_static_font_response(APP_FONT_NOTO_SANS_JP_JAPANESE_REGULAR)
        }
        "noto-sans-jp-latin-500.woff2" => {
            app_static_font_response(APP_FONT_NOTO_SANS_JP_LATIN_MEDIUM)
        }
        "noto-sans-jp-japanese-500.woff2" => {
            app_static_font_response(APP_FONT_NOTO_SANS_JP_JAPANESE_MEDIUM)
        }
        "noto-sans-jp-latin-700.woff2" => {
            app_static_font_response(APP_FONT_NOTO_SANS_JP_LATIN_BOLD)
        }
        "noto-sans-jp-japanese-700.woff2" => {
            app_static_font_response(APP_FONT_NOTO_SANS_JP_JAPANESE_BOLD)
        }
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn wasm_init_script() -> Response {
    app_static_text_response("application/javascript; charset=utf-8", WASM_INIT_JS)
}

/// Serve the wasm-bindgen JS loader from the Trunk dist directory at runtime.
async fn wasm_glue_javascript(State(state): State<AppState>) -> Response {
    let asset_path = match locate_frontend_asset(
        &state,
        FrontendBundleAsset::JavaScript,
        "wasm_glue_javascript",
    ) {
        Ok(path) => path,
        Err(detail) => return frontend_unavailable_response_detail(&detail),
    };

    match tokio::fs::read_to_string(&asset_path).await {
        Ok(content) => app_dynamic_text_response("application/javascript; charset=utf-8", content),
        Err(err) => {
            tracing::warn!(%err, path = %asset_path.display(), "failed to read frontend javascript bundle");
            frontend_unavailable_response("wasm_glue_javascript: file not found")
        }
    }
}

/// Serve the compiled WebAssembly binary from the Trunk dist directory at runtime.
async fn wasm_binary(State(state): State<AppState>) -> Response {
    let asset_path = match locate_frontend_asset(&state, FrontendBundleAsset::Wasm, "wasm_binary") {
        Ok(path) => path,
        Err(detail) => return frontend_unavailable_response_detail(&detail),
    };

    match tokio::fs::read(&asset_path).await {
        Ok(bytes) => {
            let headers = asset_response_headers("application/wasm");
            (headers, bytes).into_response()
        }
        Err(err) => {
            tracing::warn!(%err, path = %asset_path.display(), "failed to read frontend wasm bundle");
            frontend_unavailable_response("wasm_binary: file not found")
        }
    }
}

fn locate_frontend_asset(
    state: &AppState,
    asset_type: FrontendBundleAsset,
    context_name: &'static str,
) -> Result<PathBuf, String> {
    let Some(dist) = state.frontend_dist.as_deref() else {
        return Err(format!("{context_name}: frontend_dist not configured"));
    };

    let locate_result = match asset_type {
        FrontendBundleAsset::JavaScript => frontend_javascript_asset_path(dist),
        FrontendBundleAsset::Wasm => frontend_wasm_asset_path(dist),
    };

    match locate_result {
        Ok(path) => Ok(path),
        Err(err) => {
            tracing::warn!(%err, asset = ?asset_type, context_name, "failed to locate frontend bundle asset");
            Err(format!("{context_name}: file not found"))
        }
    }
}

fn frontend_javascript_asset_path(dist: &FsPath) -> io::Result<PathBuf> {
    find_frontend_bundle_asset(dist, FrontendBundleAsset::JavaScript)
}

fn frontend_wasm_asset_path(dist: &FsPath) -> io::Result<PathBuf> {
    find_frontend_bundle_asset(dist, FrontendBundleAsset::Wasm)
}

fn frontend_unavailable_response(detail: &'static str) -> Response {
    frontend_unavailable_response_detail(detail)
}

fn frontend_unavailable_response_detail(detail: &str) -> Response {
    tracing::debug!(detail, "frontend WASM assets not available");
    (
        StatusCode::SERVICE_UNAVAILABLE,
        "Web frontend assets not available. Run `cargo run -- --web` to build and serve them.",
    )
        .into_response()
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

fn app_static_text_response(content_type: &'static str, body: &'static str) -> Response {
    let response_headers = asset_response_headers(content_type);
    (response_headers, body).into_response()
}

fn app_static_font_response(body: &'static [u8]) -> Response {
    let response_headers =
        asset_response_headers_with_cache("font/woff2", "public, max-age=31536000, immutable");
    (response_headers, body).into_response()
}

fn app_dynamic_text_response(content_type: &'static str, body: String) -> Response {
    let response_headers = asset_response_headers(content_type);
    (response_headers, body).into_response()
}

fn asset_response_headers(content_type: &'static str) -> HeaderMap {
    asset_response_headers_with_cache(content_type, "no-store")
}

fn asset_response_headers_with_cache(
    content_type: &'static str,
    cache_control: &'static str,
) -> HeaderMap {
    let mut response_headers = HeaderMap::new();
    response_headers.insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
    response_headers.insert(CACHE_CONTROL, HeaderValue::from_static(cache_control));
    response_headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    response_headers.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    response_headers
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
            "default-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'; img-src 'self' data:; style-src 'self'; script-src 'self' 'wasm-unsafe-eval'; connect-src 'self'",
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

const APP_SHELL_DOCUMENT_TEMPLATE: &str = include_str!("app_assets/app.html");
const APP_STYLESHEET: &str = include_str!("app_assets/app.css");
const APP_FONT_NOTO_SANS_JP_LATIN_REGULAR: &[u8] =
    include_bytes!("app_assets/fonts/noto-sans-jp-latin-400-normal.woff2");
const APP_FONT_NOTO_SANS_JP_JAPANESE_REGULAR: &[u8] =
    include_bytes!("app_assets/fonts/noto-sans-jp-japanese-400-normal.woff2");
const APP_FONT_NOTO_SANS_JP_LATIN_MEDIUM: &[u8] =
    include_bytes!("app_assets/fonts/noto-sans-jp-latin-500-normal.woff2");
const APP_FONT_NOTO_SANS_JP_JAPANESE_MEDIUM: &[u8] =
    include_bytes!("app_assets/fonts/noto-sans-jp-japanese-500-normal.woff2");
const APP_FONT_NOTO_SANS_JP_LATIN_BOLD: &[u8] =
    include_bytes!("app_assets/fonts/noto-sans-jp-latin-700-normal.woff2");
const APP_FONT_NOTO_SANS_JP_JAPANESE_BOLD: &[u8] =
    include_bytes!("app_assets/fonts/noto-sans-jp-japanese-700-normal.woff2");
const WASM_INIT_JS: &str = "import init from \"./acp-web-frontend.js\";\n\nawait init();\n";

fn app_shell_document(csrf_token: &str) -> Html<String> {
    assert!(
        APP_SHELL_DOCUMENT_TEMPLATE.contains("__ACP_CSRF_TOKEN__"),
        "app.html must contain the __ACP_CSRF_TOKEN__ sentinel",
    );
    Html(APP_SHELL_DOCUMENT_TEMPLATE.replace("__ACP_CSRF_TOKEN__", csrf_token))
}

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

fn current_browser_session_id(headers: &HeaderMap) -> Option<String> {
    cookie_uuid_value(headers, SESSION_COOKIE_NAME)
}

fn require_admin(user: &UserRecord) -> Result<(), AppError> {
    if user.is_admin {
        Ok(())
    } else {
        Err(AppError::Forbidden("admin access required".to_string()))
    }
}

async fn auth_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<AuthStatusResponse>, AppError> {
    let (bootstrap_required, account) = state
        .workspace_repository
        .auth_status(current_browser_session_id(&headers).as_deref())
        .await?;
    Ok(Json(AuthStatusResponse {
        bootstrap_required,
        account: account
            .as_ref()
            .map(user_record_to_local_account)
            .transpose()?,
    }))
}

async fn bootstrap_register(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
    Json(request): Json<BootstrapRegistrationRequest>,
) -> Result<(StatusCode, Json<BootstrapRegistrationResponse>), AppError> {
    if !matches!(
        principal.kind,
        crate::auth::AuthenticatedPrincipalKind::BrowserSession
    ) {
        return Err(AppError::Forbidden(
            "bootstrap registration requires a browser session".to_string(),
        ));
    }
    let account = state
        .workspace_repository
        .bootstrap_local_account(&principal.id, &request.username, &request.password)
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(BootstrapRegistrationResponse { account }),
    ))
}

async fn sign_in(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
    Json(request): Json<SignInRequest>,
) -> Result<Json<SignInResponse>, AppError> {
    if !matches!(
        principal.kind,
        crate::auth::AuthenticatedPrincipalKind::BrowserSession
    ) {
        return Err(AppError::Forbidden(
            "password sign-in requires a browser session".to_string(),
        ));
    }
    let account = state
        .workspace_repository
        .sign_in_local_account(&principal.id, &request.username, &request.password)
        .await?;
    let invalidated_sessions = state
        .store
        .delete_sessions_for_owners(std::slice::from_ref(&principal.id))
        .await;
    for session_id in invalidated_sessions {
        state.reply_provider.forget_session(&session_id);
    }
    Ok(Json(SignInResponse { account }))
}

async fn list_accounts(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
) -> Result<Json<AccountListResponse>, AppError> {
    let owner = state.owner_context(principal).await?;
    require_admin(&owner.user)?;
    let accounts = state.workspace_repository.list_local_accounts().await?;
    Ok(Json(AccountListResponse {
        current_user_id: owner.user.user_id,
        accounts,
    }))
}

async fn create_account(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
    Json(request): Json<CreateAccountRequest>,
) -> Result<(StatusCode, Json<CreateAccountResponse>), AppError> {
    let owner = state.owner_context(principal).await?;
    require_admin(&owner.user)?;
    let account = state
        .workspace_repository
        .create_local_account(&request.username, &request.password, request.is_admin)
        .await?;
    Ok((StatusCode::CREATED, Json(CreateAccountResponse { account })))
}

async fn update_account(
    State(state): State<AppState>,
    Path(user_id): Path<String>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
    Json(request): Json<UpdateAccountRequest>,
) -> Result<Json<UpdateAccountResponse>, AppError> {
    let owner = state.owner_context(principal).await?;
    require_admin(&owner.user)?;
    let account = state
        .workspace_repository
        .update_local_account(
            &user_id,
            &owner.user.user_id,
            request.password.as_deref(),
            request.is_admin,
        )
        .await?;
    Ok(Json(UpdateAccountResponse { account }))
}

async fn delete_account(
    State(state): State<AppState>,
    Path(user_id): Path<String>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
) -> Result<Json<DeleteAccountResponse>, AppError> {
    let owner = state.owner_context(principal).await?;
    require_admin(&owner.user)?;
    let invalidated_browser_sessions = state
        .workspace_repository
        .delete_local_account(&user_id, &owner.user.user_id)
        .await?;
    let invalidated_sessions = state
        .store
        .delete_sessions_for_owners(&invalidated_browser_sessions)
        .await;
    for session_id in invalidated_sessions {
        state.reply_provider.forget_session(&session_id);
    }
    Ok(Json(DeleteAccountResponse { deleted: true }))
}

fn user_record_to_local_account(user: &UserRecord) -> Result<LocalAccount, AppError> {
    Ok(LocalAccount {
        user_id: user.user_id.clone(),
        username: user
            .username
            .clone()
            .ok_or_else(|| AppError::Internal("local account missing username".to_string()))?,
        is_admin: user.is_admin,
        created_at: user.created_at,
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
    Extension(principal): Extension<AuthenticatedPrincipal>,
) -> Result<Json<SessionListResponse>, AppError> {
    let owner = state.owner_context(principal).await?;
    let sessions = state.store.list_owned_sessions(&owner.principal.id).await;

    Ok(Json(SessionListResponse { sessions }))
}

async fn create_session(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
) -> Result<(StatusCode, Json<CreateSessionResponse>), AppError> {
    let owner = state.owner_context(principal).await?;
    let session = state.store.create_session(&owner.principal.id).await?;
    let session_id = session.id.clone();
    let session = match seed_startup_hint(&state, &owner.principal.id, session).await {
        Ok(session) => session,
        Err(error) => {
            if let Err(rollback_error) =
                rollback_failed_session(&state, &owner.principal.id, &session_id).await
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
    if let Err(error) = persist_session_metadata(&state, &owner.user, &session, true, None).await {
        if let Err(rollback_error) =
            rollback_failed_session(&state, &owner.principal.id, &session_id).await
        {
            return Err(AppError::Internal(format!(
                "{}; session rollback failed: {}",
                error.message(),
                rollback_error.message()
            )));
        }
        return Err(error);
    }

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
    Extension(principal): Extension<AuthenticatedPrincipal>,
) -> Result<Json<SessionResponse>, AppError> {
    let owner = state.owner_context(principal).await?;
    let session = state
        .store
        .session_snapshot(&owner.principal.id, &session_id)
        .await?;

    Ok(Json(SessionResponse { session }))
}

async fn rename_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
    Json(request): Json<RenameSessionRequest>,
) -> Result<Json<RenameSessionResponse>, AppError> {
    let owner = live_session_write_context(&state, principal, "rename").await?;
    let title = request.title.trim().to_string();
    if title.is_empty() {
        return Err(AppError::BadRequest("title must not be empty".to_string()));
    }
    if title.chars().count() > 500 {
        return Err(AppError::BadRequest(
            "title must not exceed 500 characters".to_string(),
        ));
    }
    let session = state
        .store
        .rename_session(&owner.principal.id, &session_id, title)
        .await?;
    if let Some(user) = owner.user.as_ref() {
        persist_session_metadata_for_user_best_effort(
            &state, user, &session, false, None, "rename",
        )
        .await;
    }

    Ok(Json(RenameSessionResponse { session }))
}

async fn delete_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
) -> Result<Json<DeleteSessionResponse>, AppError> {
    let owner = live_session_write_context(&state, principal, "delete").await?;
    let snapshot = state
        .store
        .session_snapshot(&owner.principal.id, &session_id)
        .await?;
    state
        .store
        .delete_session(&owner.principal.id, &session_id)
        .await?;
    state.reply_provider.forget_session(&session_id);
    if let Some(user) = owner.user.as_ref() {
        persist_session_metadata_for_user_best_effort(
            &state,
            user,
            &snapshot,
            false,
            Some("deleted"),
            "delete",
        )
        .await;
    }

    Ok(Json(DeleteSessionResponse { deleted: true }))
}

async fn get_session_history(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
) -> Result<Json<SessionHistoryResponse>, AppError> {
    let owner = state.owner_context(principal).await?;
    let messages = state
        .store
        .session_history(&owner.principal.id, &session_id)
        .await?;

    Ok(Json(SessionHistoryResponse {
        session_id,
        messages,
    }))
}

async fn post_message(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
    Json(request): Json<PromptRequest>,
) -> Result<Json<PromptResponse>, AppError> {
    let owner = live_session_write_context(&state, principal, "submit_prompt").await?;
    let pending = state
        .store
        .submit_prompt(&owner.principal.id, &session_id, request.text)
        .await?;
    let snapshot_result = state
        .store
        .session_snapshot(&owner.principal.id, &session_id)
        .await;
    if let Some(user) = owner.user.as_ref() {
        persist_prompt_snapshot_best_effort(&state, user, &session_id, snapshot_result).await;
    }
    dispatch_assistant_request(state.reply_provider.clone(), pending);

    Ok(Json(PromptResponse { accepted: true }))
}

async fn close_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
) -> Result<Json<CloseSessionResponse>, AppError> {
    let owner = live_session_write_context(&state, principal, "close").await?;
    let session = state
        .store
        .close_session(&owner.principal.id, &session_id)
        .await?;
    state.reply_provider.forget_session(&session_id);
    if let Some(user) = owner.user.as_ref() {
        persist_session_metadata_for_user_best_effort(
            &state,
            user,
            &session,
            false,
            Some("closed"),
            "close",
        )
        .await;
    }

    Ok(Json(CloseSessionResponse { session }))
}

async fn cancel_turn(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
) -> Result<Json<CancelTurnResponse>, AppError> {
    let owner = state.owner_context(principal).await?;
    let cancelled = state
        .store
        .cancel_active_turn(&owner.principal.id, &session_id)
        .await?;

    Ok(Json(CancelTurnResponse { cancelled }))
}

async fn resolve_permission(
    State(state): State<AppState>,
    Path((session_id, request_id)): Path<(String, String)>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
    Json(request): Json<ResolvePermissionRequest>,
) -> Result<Json<ResolvePermissionResponse>, AppError> {
    let owner = state.owner_context(principal).await?;
    let resolution = state
        .store
        .resolve_permission(
            &owner.principal.id,
            &session_id,
            &request_id,
            request.decision,
        )
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
    Extension(principal): Extension<AuthenticatedPrincipal>,
) -> Result<Json<SlashCompletionsResponse>, AppError> {
    let owner = state.owner_context(principal).await?;
    let response_future = resolve_slash_completions(
        &state.store,
        &owner.principal.id,
        &query.session_id,
        &query.prefix,
    );
    let response = response_future.await?;

    Ok(Json(response))
}

async fn stream_session_events(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let owner = state.owner_context(principal).await?;
    let (snapshot, receiver) = state
        .store
        .session_events(&owner.principal.id, &session_id)
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

impl From<WorkspaceStoreError> for AppError {
    fn from(error: WorkspaceStoreError) -> Self {
        match error {
            WorkspaceStoreError::Io(_) | WorkspaceStoreError::Database(_) => {
                tracing::error!(error = %error.message(), "workspace store operation failed");
                Self::Internal("internal server error".to_string())
            }
            WorkspaceStoreError::Unauthorized(message) => Self::Unauthorized(message),
            WorkspaceStoreError::NotFound(message) => Self::NotFound(message),
            WorkspaceStoreError::Conflict(message) => Self::Conflict(message),
            WorkspaceStoreError::Validation(message) => Self::BadRequest(message),
        }
    }
}

#[cfg(test)]
mod tests;
