use std::{convert::Infallible, fmt::Display, path::PathBuf, sync::Arc, time::Duration};

#[cfg(test)]
use crate::contract_sessions::RenameSessionRequest;
use axum::{
    Router,
    body::Body,
    extract::FromRequestParts,
    handler::Handler,
    http::{HeaderMap, Request, StatusCode, request::Parts},
    response::{IntoResponse, Response},
    routing::{MethodRouter, get_service, patch_service, post_service},
};
use tower::util::BoxCloneSyncService;
#[cfg(test)]
use uuid::Uuid;

use crate::contract_sessions::RenameSessionRequest as RenameSessionBody;
#[cfg(test)]
use crate::sessions::TurnHandle;
use crate::workspace_checkout::FsWorkspaceCheckoutManager;
#[cfg(test)]
use crate::workspace_checkout::{
    PreparedWorkspaceCheckout, WorkspaceCheckoutError, WorkspaceCheckoutManager,
};
#[cfg(test)]
use crate::workspace_store::SqliteWorkspaceRepository;
use crate::{
    auth::{
        AuthError, AuthenticatedPrincipal, CSRF_COOKIE_NAME, SESSION_COOKIE_NAME,
        authorize_request, cookie_value,
    },
    contract_accounts::{
        BootstrapRegistrationRequest, CreateAccountRequest, SignInRequest, UpdateAccountRequest,
    },
    contract_health::ErrorResponse,
    contract_messages::PromptRequest,
    contract_permissions::ResolvePermissionRequest,
    contract_workspaces::{CreateWorkspaceRequest, UpdateWorkspaceRequest},
    mock_client::{MockClient, MockClientError, ReplyProvider},
    sessions::{SessionStore, SessionStoreError},
    workspace_checkout::DynWorkspaceCheckoutManager,
    workspace_records::{UserRecord, WorkspaceStoreError},
    workspace_repository::WorkspaceRepository,
};

mod account_api;
mod account_service;
mod assets;
mod connection;
mod session_api;
mod session_service;
mod workspace_api;
mod workspace_service;

use self::account_api::{
    auth_status, bootstrap_register, create_account, delete_account, list_accounts, sign_in,
    sign_out, update_account,
};
use self::assets::{SlashCompletionsQuery, install_frontend_routes};
pub use self::connection::serve_with_shutdown;
#[cfg(test)]
use self::session_api::create_session;
use self::session_api::{
    cancel_turn, close_session, delete_session, get_session, get_session_history,
    get_slash_completions, list_sessions, parse_json_body, post_message, rename_session,
    resolve_permission, stream_session_events,
};
#[cfg(test)]
use self::session_service::{
    persist_prompt_snapshot_best_effort, persist_session_metadata_best_effort,
};
use self::workspace_api::{
    bootstrap_workspace, create_workspace, create_workspace_session, delete_workspace,
    get_workspace, list_workspace_branches, list_workspace_sessions, list_workspaces,
    update_workspace,
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
    checkout_manager: DynWorkspaceCheckoutManager,
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
        let store = Arc::new(SessionStore::new(config.session_cap));
        let reply_provider = Arc::new(MockClient::new(config.acp_server)?);
        let checkout_manager: DynWorkspaceCheckoutManager =
            Arc::new(FsWorkspaceCheckoutManager::new(config.state_dir));

        Ok(Self::with_services(
            store,
            workspace_repository,
            reply_provider,
            checkout_manager,
            config.startup_hints,
            config.frontend_dist,
        ))
    }

    pub fn with_services(
        store: Arc<SessionStore>,
        workspace_repository: Arc<dyn WorkspaceRepository>,
        reply_provider: Arc<dyn ReplyProvider>,
        checkout_manager: DynWorkspaceCheckoutManager,
        startup_hints: bool,
        frontend_dist: Option<PathBuf>,
    ) -> Self {
        Self {
            store,
            workspace_repository,
            reply_provider,
            checkout_manager,
            startup_hints,
            frontend_dist: frontend_dist.map(Arc::new),
        }
    }

    #[cfg(test)]
    pub fn with_dependencies(
        store: Arc<SessionStore>,
        reply_provider: Arc<dyn ReplyProvider>,
    ) -> Self {
        Self::with_workspace_repository_and_checkout_manager(
            store,
            new_ephemeral_workspace_repository(),
            reply_provider,
            test_checkout_manager(),
        )
    }

    #[cfg(test)]
    pub fn with_workspace_repository(
        store: Arc<SessionStore>,
        workspace_repository: Arc<dyn WorkspaceRepository>,
        reply_provider: Arc<dyn ReplyProvider>,
    ) -> Self {
        Self::with_workspace_repository_and_checkout_manager(
            store,
            workspace_repository,
            reply_provider,
            test_checkout_manager(),
        )
    }

    #[cfg(test)]
    pub fn with_workspace_repository_and_checkout_manager(
        store: Arc<SessionStore>,
        workspace_repository: Arc<dyn WorkspaceRepository>,
        reply_provider: Arc<dyn ReplyProvider>,
        checkout_manager: DynWorkspaceCheckoutManager,
    ) -> Self {
        Self::with_services(
            store,
            workspace_repository,
            reply_provider,
            checkout_manager,
            false,
            None,
        )
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
        let live_owner_id = live_owner_id_for_principal(&principal, &user);
        Ok(OwnerContext {
            user,
            live_owner_id,
        })
    }
}

pub fn app(state: AppState) -> Router {
    install_frontend_routes(Router::new(), state.clone())
        .route(
            "/api/v1/auth/status",
            get_route(&state, route_handlers::auth_status_handler),
        )
        .merge(read_api_routes(state.clone()))
        .merge(write_api_routes(state))
}

#[derive(Debug, Clone)]
struct OwnerContext {
    user: UserRecord,
    live_owner_id: String,
}

fn live_owner_id_for_principal(principal: &AuthenticatedPrincipal, user: &UserRecord) -> String {
    match principal.kind {
        crate::auth::AuthenticatedPrincipalKind::Bearer => live_owner_id_for_bearer(principal),
        crate::auth::AuthenticatedPrincipalKind::BrowserSession => {
            live_owner_id_for_browser_user(user)
        }
    }
}

fn live_owner_id_for_bearer(principal: &AuthenticatedPrincipal) -> String {
    format!("bearer:{}", principal.id)
}

fn live_owner_id_for_browser_user(user: &UserRecord) -> String {
    live_owner_id_for_browser_user_id(&user.user_id)
}

fn live_owner_id_for_browser_user_id(user_id: &str) -> String {
    format!("browser-user:{user_id}")
}

type BoxedRouteService = BoxCloneSyncService<Request<Body>, Response, Infallible>;

#[derive(Clone)]
struct ReadPrincipal(AuthenticatedPrincipal);

#[derive(Clone)]
struct WritePrincipal(AuthenticatedPrincipal);

fn auth_principal(
    headers: &HeaderMap,
    requires_csrf: bool,
) -> Result<AuthenticatedPrincipal, AppError> {
    authorize_request(headers, requires_csrf).map_err(AppError::from)
}

impl<S> FromRequestParts<S> for ReadPrincipal
where
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        auth_principal(&parts.headers, false).map(Self)
    }
}

impl<S> FromRequestParts<S> for WritePrincipal
where
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        auth_principal(&parts.headers, true).map(Self)
    }
}

mod route_handlers {
    use axum::{
        Json,
        body::Bytes,
        extract::{Extension, Path, Query, State},
        http::HeaderMap,
        response::{IntoResponse, Response},
    };

    use super::{
        AppError, AppState, BootstrapRegistrationRequest, CreateAccountRequest,
        CreateWorkspaceRequest, PromptRequest, ReadPrincipal, RenameSessionBody,
        ResolvePermissionRequest, SignInRequest, SlashCompletionsQuery, UpdateAccountRequest,
        UpdateWorkspaceRequest, WritePrincipal, auth_status, bootstrap_register,
        bootstrap_workspace, cancel_turn, close_session, create_account, create_workspace,
        create_workspace_session, delete_account, delete_session, delete_workspace, get_session,
        get_session_history, get_slash_completions, get_workspace, list_accounts, list_sessions,
        list_workspace_branches, list_workspace_sessions, list_workspaces, parse_json_body,
        post_message, rename_session, resolve_permission, sign_in, sign_out, stream_session_events,
        update_account, update_workspace,
    };

    pub(super) async fn auth_status_handler(
        State(state): State<AppState>,
        headers: HeaderMap,
    ) -> Result<Response, AppError> {
        auth_status(State(state), headers)
            .await
            .map(IntoResponse::into_response)
    }

    pub(super) async fn list_accounts_handler(
        State(state): State<AppState>,
        ReadPrincipal(principal): ReadPrincipal,
    ) -> Result<Response, AppError> {
        list_accounts(State(state), Extension(principal))
            .await
            .map(IntoResponse::into_response)
    }

    pub(super) async fn list_sessions_handler(
        State(state): State<AppState>,
        ReadPrincipal(principal): ReadPrincipal,
    ) -> Result<Response, AppError> {
        list_sessions(State(state), Extension(principal))
            .await
            .map(IntoResponse::into_response)
    }

    pub(super) async fn list_workspaces_handler(
        State(state): State<AppState>,
        ReadPrincipal(principal): ReadPrincipal,
    ) -> Result<Response, AppError> {
        list_workspaces(State(state), Extension(principal))
            .await
            .map(IntoResponse::into_response)
    }

    pub(super) async fn get_workspace_handler(
        State(state): State<AppState>,
        Path(workspace_id): Path<String>,
        ReadPrincipal(principal): ReadPrincipal,
    ) -> Result<Response, AppError> {
        get_workspace(State(state), Path(workspace_id), Extension(principal))
            .await
            .map(IntoResponse::into_response)
    }

    pub(super) async fn list_workspace_branches_handler(
        State(state): State<AppState>,
        Path(workspace_id): Path<String>,
        ReadPrincipal(principal): ReadPrincipal,
    ) -> Result<Response, AppError> {
        list_workspace_branches(State(state), Path(workspace_id), Extension(principal))
            .await
            .map(IntoResponse::into_response)
    }

    pub(super) async fn list_workspace_sessions_handler(
        State(state): State<AppState>,
        Path(workspace_id): Path<String>,
        ReadPrincipal(principal): ReadPrincipal,
    ) -> Result<Response, AppError> {
        list_workspace_sessions(State(state), Path(workspace_id), Extension(principal))
            .await
            .map(IntoResponse::into_response)
    }

    pub(super) async fn get_session_handler(
        State(state): State<AppState>,
        Path(session_id): Path<String>,
        ReadPrincipal(principal): ReadPrincipal,
    ) -> Result<Response, AppError> {
        get_session(State(state), Path(session_id), Extension(principal))
            .await
            .map(IntoResponse::into_response)
    }

    pub(super) async fn get_session_history_handler(
        State(state): State<AppState>,
        Path(session_id): Path<String>,
        ReadPrincipal(principal): ReadPrincipal,
    ) -> Result<Response, AppError> {
        get_session_history(State(state), Path(session_id), Extension(principal))
            .await
            .map(IntoResponse::into_response)
    }

    pub(super) async fn stream_session_events_handler(
        State(state): State<AppState>,
        Path(session_id): Path<String>,
        ReadPrincipal(principal): ReadPrincipal,
    ) -> Result<Response, AppError> {
        stream_session_events(State(state), Path(session_id), Extension(principal))
            .await
            .map(IntoResponse::into_response)
    }

    pub(super) async fn get_slash_completions_handler(
        State(state): State<AppState>,
        Query(query): Query<SlashCompletionsQuery>,
        ReadPrincipal(principal): ReadPrincipal,
    ) -> Result<Response, AppError> {
        get_slash_completions(State(state), Query(query), Extension(principal))
            .await
            .map(IntoResponse::into_response)
    }

    pub(super) async fn sign_in_handler(
        State(state): State<AppState>,
        WritePrincipal(principal): WritePrincipal,
        Json(request): Json<SignInRequest>,
    ) -> Result<Response, AppError> {
        sign_in(State(state), Extension(principal), Json(request))
            .await
            .map(IntoResponse::into_response)
    }

    pub(super) async fn sign_out_handler(
        State(state): State<AppState>,
        WritePrincipal(principal): WritePrincipal,
    ) -> Result<Response, AppError> {
        sign_out(State(state), Extension(principal))
            .await
            .map(IntoResponse::into_response)
    }

    pub(super) async fn bootstrap_register_handler(
        State(state): State<AppState>,
        WritePrincipal(principal): WritePrincipal,
        Json(request): Json<BootstrapRegistrationRequest>,
    ) -> Result<Response, AppError> {
        bootstrap_register(State(state), Extension(principal), Json(request))
            .await
            .map(IntoResponse::into_response)
    }

    pub(super) async fn create_workspace_handler(
        State(state): State<AppState>,
        WritePrincipal(principal): WritePrincipal,
        Json(request): Json<CreateWorkspaceRequest>,
    ) -> Result<Response, AppError> {
        create_workspace(State(state), Extension(principal), Json(request))
            .await
            .map(IntoResponse::into_response)
    }

    pub(super) async fn bootstrap_workspace_handler(
        State(state): State<AppState>,
        WritePrincipal(principal): WritePrincipal,
    ) -> Result<Response, AppError> {
        bootstrap_workspace(State(state), Extension(principal))
            .await
            .map(IntoResponse::into_response)
    }

    pub(super) async fn create_account_handler(
        State(state): State<AppState>,
        WritePrincipal(principal): WritePrincipal,
        Json(request): Json<CreateAccountRequest>,
    ) -> Result<Response, AppError> {
        create_account(State(state), Extension(principal), Json(request))
            .await
            .map(IntoResponse::into_response)
    }

    pub(super) async fn update_account_handler(
        State(state): State<AppState>,
        Path(user_id): Path<String>,
        WritePrincipal(principal): WritePrincipal,
        headers: HeaderMap,
        body: Bytes,
    ) -> Result<Response, AppError> {
        let request = parse_json_body::<UpdateAccountRequest>(&headers, &body)?;
        update_account(
            State(state),
            Path(user_id),
            Extension(principal),
            Json(request),
        )
        .await
        .map(IntoResponse::into_response)
    }

    pub(super) async fn delete_account_handler(
        State(state): State<AppState>,
        Path(user_id): Path<String>,
        WritePrincipal(principal): WritePrincipal,
    ) -> Result<Response, AppError> {
        delete_account(State(state), Path(user_id), Extension(principal))
            .await
            .map(IntoResponse::into_response)
    }

    pub(super) async fn update_workspace_handler(
        State(state): State<AppState>,
        Path(workspace_id): Path<String>,
        WritePrincipal(principal): WritePrincipal,
        headers: HeaderMap,
        body: Bytes,
    ) -> Result<Response, AppError> {
        let request = parse_json_body::<UpdateWorkspaceRequest>(&headers, &body)?;
        update_workspace(
            State(state),
            Path(workspace_id),
            Extension(principal),
            Json(request),
        )
        .await
        .map(IntoResponse::into_response)
    }

    pub(super) async fn delete_workspace_handler(
        State(state): State<AppState>,
        Path(workspace_id): Path<String>,
        WritePrincipal(principal): WritePrincipal,
    ) -> Result<Response, AppError> {
        delete_workspace(State(state), Path(workspace_id), Extension(principal))
            .await
            .map(IntoResponse::into_response)
    }

    pub(super) async fn create_workspace_session_handler(
        State(state): State<AppState>,
        Path(workspace_id): Path<String>,
        WritePrincipal(principal): WritePrincipal,
        body: Bytes,
    ) -> Result<Response, AppError> {
        create_workspace_session(State(state), Path(workspace_id), Extension(principal), body)
            .await
            .map(IntoResponse::into_response)
    }

    pub(super) async fn rename_session_handler(
        State(state): State<AppState>,
        Path(session_id): Path<String>,
        WritePrincipal(principal): WritePrincipal,
        headers: HeaderMap,
        body: Bytes,
    ) -> Result<Response, AppError> {
        let request = parse_json_body::<RenameSessionBody>(&headers, &body)?;
        rename_session(
            State(state),
            Path(session_id),
            Extension(principal),
            Json(request),
        )
        .await
        .map(IntoResponse::into_response)
    }

    pub(super) async fn delete_session_handler(
        State(state): State<AppState>,
        Path(session_id): Path<String>,
        WritePrincipal(principal): WritePrincipal,
    ) -> Result<Response, AppError> {
        delete_session(State(state), Path(session_id), Extension(principal))
            .await
            .map(IntoResponse::into_response)
    }

    pub(super) async fn post_message_handler(
        State(state): State<AppState>,
        Path(session_id): Path<String>,
        WritePrincipal(principal): WritePrincipal,
        headers: HeaderMap,
        body: Bytes,
    ) -> Result<Response, AppError> {
        let request = parse_json_body::<PromptRequest>(&headers, &body)?;
        post_message(
            State(state),
            Path(session_id),
            Extension(principal),
            Json(request),
        )
        .await
        .map(IntoResponse::into_response)
    }

    pub(super) async fn cancel_turn_handler(
        State(state): State<AppState>,
        Path(session_id): Path<String>,
        WritePrincipal(principal): WritePrincipal,
    ) -> Result<Response, AppError> {
        cancel_turn(State(state), Path(session_id), Extension(principal))
            .await
            .map(IntoResponse::into_response)
    }

    pub(super) async fn resolve_permission_handler(
        State(state): State<AppState>,
        Path((session_id, request_id)): Path<(String, String)>,
        WritePrincipal(principal): WritePrincipal,
        headers: HeaderMap,
        body: Bytes,
    ) -> Result<Response, AppError> {
        let request = parse_json_body::<ResolvePermissionRequest>(&headers, &body)?;
        resolve_permission(
            State(state),
            Path((session_id, request_id)),
            Extension(principal),
            Json(request),
        )
        .await
        .map(IntoResponse::into_response)
    }

    pub(super) async fn close_session_handler(
        State(state): State<AppState>,
        Path(session_id): Path<String>,
        WritePrincipal(principal): WritePrincipal,
    ) -> Result<Response, AppError> {
        close_session(State(state), Path(session_id), Extension(principal))
            .await
            .map(IntoResponse::into_response)
    }
}

pub(super) fn get_route<H, T>(state: &AppState, handler: H) -> MethodRouter
where
    H: Handler<T, AppState> + Clone + Send + Sync + 'static,
    T: 'static,
{
    get_service(boxed_handler_service(state.clone(), handler))
}

fn post_route<H, T>(state: &AppState, handler: H) -> MethodRouter
where
    H: Handler<T, AppState> + Clone + Send + Sync + 'static,
    T: 'static,
{
    post_service(boxed_handler_service(state.clone(), handler))
}

fn patch_route<H, T>(state: &AppState, handler: H) -> MethodRouter
where
    H: Handler<T, AppState> + Clone + Send + Sync + 'static,
    T: 'static,
{
    patch_service(boxed_handler_service(state.clone(), handler))
}

fn boxed_handler_service<H, T>(state: AppState, handler: H) -> BoxedRouteService
where
    H: Handler<T, AppState> + Clone + Send + Sync + 'static,
    T: 'static,
{
    BoxCloneSyncService::new(handler.with_state(state))
}

fn read_api_routes(state: AppState) -> Router {
    Router::new()
        .route(
            "/api/v1/accounts",
            get_route(&state, route_handlers::list_accounts_handler),
        )
        .route(
            "/api/v1/sessions",
            get_route(&state, route_handlers::list_sessions_handler),
        )
        .route(
            "/api/v1/workspaces",
            get_route(&state, route_handlers::list_workspaces_handler),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}",
            get_route(&state, route_handlers::get_workspace_handler),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}/branches",
            get_route(&state, route_handlers::list_workspace_branches_handler),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}/sessions",
            get_route(&state, route_handlers::list_workspace_sessions_handler),
        )
        .route(
            "/api/v1/sessions/{session_id}",
            get_route(&state, route_handlers::get_session_handler),
        )
        .route(
            "/api/v1/sessions/{session_id}/history",
            get_route(&state, route_handlers::get_session_history_handler),
        )
        .route(
            "/api/v1/sessions/{session_id}/events",
            get_route(&state, route_handlers::stream_session_events_handler),
        )
        .route(
            "/api/v1/completions/slash",
            get_route(&state, route_handlers::get_slash_completions_handler),
        )
}

fn write_api_routes(state: AppState) -> Router {
    auth_write_routes(state.clone())
        .merge(account_write_routes(state.clone()))
        .merge(workspace_write_routes(state.clone()))
        .merge(session_write_routes(state))
}

fn auth_write_routes(state: AppState) -> Router {
    Router::new()
        .route(
            "/api/v1/auth/sign-in",
            post_route(&state, route_handlers::sign_in_handler),
        )
        .route(
            "/api/v1/auth/sign-out",
            post_route(&state, route_handlers::sign_out_handler),
        )
        .route(
            "/api/v1/bootstrap/register",
            post_route(&state, route_handlers::bootstrap_register_handler),
        )
}

fn account_write_routes(state: AppState) -> Router {
    Router::new()
        .route(
            "/api/v1/accounts",
            post_route(&state, route_handlers::create_account_handler),
        )
        .route(
            "/api/v1/accounts/{user_id}",
            patch_route(&state, route_handlers::update_account_handler).delete_service(
                boxed_handler_service(state.clone(), route_handlers::delete_account_handler),
            ),
        )
}

fn workspace_write_routes(state: AppState) -> Router {
    Router::new()
        .route(
            "/api/v1/workspaces",
            post_route(&state, route_handlers::create_workspace_handler),
        )
        .route(
            "/api/v1/workspaces/bootstrap",
            post_route(&state, route_handlers::bootstrap_workspace_handler),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}",
            patch_route(&state, route_handlers::update_workspace_handler).delete_service(
                boxed_handler_service(state.clone(), route_handlers::delete_workspace_handler),
            ),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}/sessions",
            post_route(&state, route_handlers::create_workspace_session_handler),
        )
}

fn session_write_routes(state: AppState) -> Router {
    Router::new()
        .route(
            "/api/v1/sessions/{session_id}",
            patch_route(&state, route_handlers::rename_session_handler).delete_service(
                boxed_handler_service(state.clone(), route_handlers::delete_session_handler),
            ),
        )
        .route(
            "/api/v1/sessions/{session_id}/messages",
            post_route(&state, route_handlers::post_message_handler),
        )
        .route(
            "/api/v1/sessions/{session_id}/cancel",
            post_route(&state, route_handlers::cancel_turn_handler),
        )
        .route(
            "/api/v1/sessions/{session_id}/permissions/{request_id}",
            post_route(&state, route_handlers::resolve_permission_handler),
        )
        .route(
            "/api/v1/sessions/{session_id}/close",
            post_route(&state, route_handlers::close_session_handler),
        )
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

#[cfg(test)]
fn test_checkout_manager() -> DynWorkspaceCheckoutManager {
    Arc::new(TestWorkspaceCheckoutManager)
}

#[cfg(test)]
#[derive(Debug, Default)]
struct TestWorkspaceCheckoutManager;

#[cfg(test)]
fn test_checkout_path(checkout_relpath: &str) -> PathBuf {
    std::env::temp_dir().join(checkout_relpath)
}

#[cfg(test)]
fn reset_test_checkout_dir(working_dir: &std::path::Path) -> Result<(), WorkspaceCheckoutError> {
    if working_dir.exists() {
        std::fs::remove_dir_all(working_dir).map_err(|error| {
            WorkspaceCheckoutError::Io(format!("clearing test checkout directory failed: {error}"))
        })?;
    }
    std::fs::create_dir_all(working_dir).map_err(|error| {
        WorkspaceCheckoutError::Io(format!("creating test checkout directory failed: {error}"))
    })?;
    Ok(())
}

#[cfg(test)]
#[async_trait::async_trait]
impl WorkspaceCheckoutManager for TestWorkspaceCheckoutManager {
    async fn prepare_checkout(
        &self,
        _workspace: &crate::workspace_records::WorkspaceRecord,
        session_id: &str,
        checkout_ref_override: Option<&str>,
    ) -> Result<PreparedWorkspaceCheckout, WorkspaceCheckoutError> {
        let checkout_relpath = format!("session-checkouts/{session_id}");
        let working_dir = test_checkout_path(&checkout_relpath);
        reset_test_checkout_dir(&working_dir)?;
        Ok(PreparedWorkspaceCheckout {
            checkout_relpath,
            checkout_ref: checkout_ref_override.map(str::to_string),
            checkout_commit_sha: Some("test-commit".to_string()),
            working_dir,
        })
    }

    fn resolve_checkout_path(&self, checkout_relpath: &str) -> Option<PathBuf> {
        Some(test_checkout_path(checkout_relpath))
    }

    async fn list_branches(
        &self,
        _workspace: &crate::workspace_records::WorkspaceRecord,
    ) -> Result<Vec<crate::contract_workspaces::WorkspaceBranch>, WorkspaceCheckoutError> {
        Ok(vec![
            crate::contract_workspaces::WorkspaceBranch {
                name: "main".to_string(),
                ref_name: "refs/heads/main".to_string(),
            },
            crate::contract_workspaces::WorkspaceBranch {
                name: "release".to_string(),
                ref_name: "refs/heads/release".to_string(),
            },
        ])
    }
}

#[derive(Debug)]
pub enum AppError {
    Unauthorized(String),
    Forbidden(String),
    NotFound(String),
    BadRequest(String),
    UnsupportedMediaType(String),
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
            Self::UnsupportedMediaType(_) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
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
            Self::UnsupportedMediaType(message) => message,
            Self::Conflict(message) => message,
            Self::TooManyRequests(message) => message,
            Self::Internal(message) => message,
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status_code();
        let message = match &self {
            Self::Internal(message) => {
                tracing::error!(error = %message, "request failed with internal error");
                "internal server error".to_string()
            }
            _ => self.message().to_string(),
        };
        (status, axum::Json(ErrorResponse { error: message })).into_response()
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
