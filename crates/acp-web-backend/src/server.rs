use std::{fmt::Display, path::PathBuf, sync::Arc, time::Duration};

#[cfg(test)]
use crate::contract_sessions::RenameSessionRequest;
#[cfg(test)]
use axum::{
    Json,
    extract::{Query, State},
    http::HeaderMap,
};
use axum::{
    Router,
    extract::Request,
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, patch, post},
};
#[cfg(test)]
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
    contract_health::ErrorResponse,
    mock_client::{MockClient, MockClientError, ReplyProvider},
    sessions::{SessionStore, SessionStoreError},
    workspace_records::{UserRecord, WorkspaceStoreError},
    workspace_repository::WorkspaceRepository,
};

mod account_api;
mod account_service;
mod assets;
mod connection;
mod session_api;
mod session_service;

use self::account_api::{
    auth_status, bootstrap_register, create_account, delete_account, list_accounts, sign_in,
    sign_out, update_account,
};
use self::assets::install_frontend_routes;
pub use self::connection::serve_with_shutdown;
use self::session_api::{
    cancel_turn, close_session, create_session, delete_session, get_session, get_session_history,
    get_slash_completions, list_sessions, post_message, rename_session, resolve_permission,
    stream_session_events,
};
#[cfg(test)]
use self::session_service::{
    persist_prompt_snapshot_best_effort, persist_session_metadata_best_effort,
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
    install_frontend_routes(Router::new())
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
        .route("/api/v1/auth/sign-out", post(sign_out))
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
            axum::Json(ErrorResponse {
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
