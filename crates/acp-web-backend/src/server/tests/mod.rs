use super::{assets::*, connection::*, *};
use crate::contract_accounts::{
    AuthStatusResponse, BootstrapRegistrationRequest, CreateAccountRequest, LocalAccount,
    SignInRequest, UpdateAccountRequest,
};
use crate::contract_messages::PromptRequest;
use crate::contract_sessions::{SessionSnapshot, SessionStatus};
use crate::contract_workspaces::{CreateWorkspaceRequest, UpdateWorkspaceRequest};
use crate::mock_client::{MockClientError, ReplyFuture, ReplyResult};
use crate::support::frontend::{FrontendBundleAsset, frontend_bundle_file_name};
use crate::support::http::build_http_client_for_url;
use crate::workspace_records::{
    DurableSessionSnapshotRecord, SessionMetadataRecord, UserRecord, WorkspaceRecord,
};
use crate::workspace_repository::WorkspaceRepository;
use crate::workspace_store::SqliteWorkspaceRepository;
use async_trait::async_trait;
use axum::{
    body::{Body, to_bytes},
    extract::{Extension, Path},
    http::{
        HeaderValue,
        header::{CACHE_CONTROL, CONTENT_TYPE, COOKIE, SET_COOKIE},
    },
    response::Response,
};
use std::sync::{Arc as StdArc, Mutex};
use tokio::time::timeout;
use tokio_rustls::TlsAcceptor;
use tower::ServiceExt;

mod app_state;
mod assets;
mod connections;
mod routing_auth;
mod session_routes;
mod support_behaviors;
mod workspace_routes;

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
        std::fs::write(
            dir.join(frontend_bundle_file_name(
                "test",
                FrontendBundleAsset::JavaScript,
            )),
            b"// stub js loader",
        )
        .expect("stub JS should be writable");
    }
    if include_wasm {
        std::fs::write(
            dir.join(frontend_bundle_file_name("test", FrontendBundleAsset::Wasm)),
            b"\x00asm\x01\x00\x00\x00", // minimal valid WASM header
        )
        .expect("stub WASM should be writable");
    }
    dir
}

fn write_temp_frontend_dist_with_unreadable_javascript() -> std::path::PathBuf {
    let dir = write_temp_frontend_dist_with(false, true);
    std::fs::create_dir(dir.join(frontend_bundle_file_name(
        "test",
        FrontendBundleAsset::JavaScript,
    )))
    .expect("stub unreadable JS directory should be creatable");
    dir
}

fn write_temp_frontend_dist_with_unreadable_wasm() -> std::path::PathBuf {
    let dir = write_temp_frontend_dist_with(true, false);
    std::fs::create_dir(dir.join(frontend_bundle_file_name("test", FrontendBundleAsset::Wasm)))
        .expect("stub unreadable WASM directory should be creatable");
    dir
}

fn test_state_with_frontend_dist(dist: std::path::PathBuf) -> AppState {
    AppState {
        store: Arc::new(SessionStore::new(1)),
        workspace_repository: new_ephemeral_workspace_repository(),
        reply_provider: Arc::new(StaticReplyProvider {
            reply: String::new(),
        }),
        startup_hints: false,
        frontend_dist: Some(Arc::new(dist)),
    }
}

fn metadata_test_workspace_store() -> Arc<SqliteWorkspaceRepository> {
    Arc::new(
        SqliteWorkspaceRepository::new(
            std::env::temp_dir()
                .join(format!(
                    "acp-server-route-metadata-{}",
                    uuid::Uuid::new_v4().simple()
                ))
                .join("db.sqlite"),
        )
        .expect("workspace repository should initialize"),
    )
}

struct MetadataTestContext {
    store: Arc<SessionStore>,
    workspace_repository: Arc<SqliteWorkspaceRepository>,
    state: AppState,
    live_owner_id: String,
    principal: Extension<AuthenticatedPrincipal>,
    user: UserRecord,
}

async fn metadata_test_context() -> MetadataTestContext {
    let store = Arc::new(SessionStore::new(4));
    let workspace_repository = metadata_test_workspace_store();
    let state = AppState::with_workspace_repository(
        store.clone(),
        workspace_repository.clone(),
        Arc::new(TrackingReplyProvider {
            forgotten_sessions: StdArc::new(Mutex::new(Vec::new())),
        }),
    );
    let headers = bearer_headers("alice");
    let user = materialized_user_for_headers(workspace_repository.as_ref(), &headers).await;

    MetadataTestContext {
        store,
        workspace_repository,
        state,
        live_owner_id: "alice".to_string(),
        principal: bearer_principal("alice"),
        user,
    }
}

async fn browser_metadata_test_context() -> MetadataTestContext {
    let store = Arc::new(SessionStore::new(4));
    let workspace_repository = metadata_test_workspace_store();
    let state = AppState::with_workspace_repository(
        store.clone(),
        workspace_repository.clone(),
        Arc::new(TrackingReplyProvider {
            forgotten_sessions: StdArc::new(Mutex::new(Vec::new())),
        }),
    );
    let shell = app_entrypoint(HeaderMap::new()).await;
    let mut headers = browser_cookie_headers(&shell);
    let body = to_bytes(shell.into_body(), usize::MAX)
        .await
        .expect("entrypoint body should be readable");
    let body = String::from_utf8(body.to_vec()).expect("entrypoint body should be UTF-8");
    let csrf_token = extract_meta_content(&body, "acp-csrf-token");
    headers.insert("x-csrf-token", HeaderValue::from_str(&csrf_token).unwrap());
    let principal = authorize_request(&headers, true).expect("browser headers should authorize");
    let _registered = bootstrap_register(
        State(state.clone()),
        Extension(principal.clone()),
        Json(BootstrapRegistrationRequest {
            username: "admin".to_string(),
            password: "password123".to_string(),
        }),
    )
    .await
    .expect("bootstrap registration should succeed");
    let user = materialized_user_for_headers(workspace_repository.as_ref(), &headers).await;

    MetadataTestContext {
        store,
        workspace_repository,
        state,
        live_owner_id: principal.id.clone(),
        principal: Extension(principal),
        user,
    }
}

async fn assert_session_routes_persist_owner_scoped_metadata(context: &MetadataTestContext) {
    let (created, created_metadata) = create_persisted_session(context).await;

    assert_eq!(created_metadata.owner_user_id, context.user.user_id);
    assert_eq!(created_metadata.status, "active");
    assert!(!created_metadata.workspace_id.is_empty());

    let (renamed, renamed_metadata) = rename_persisted_session(context, &created.id).await;

    assert_eq!(renamed.title, "Renamed session");
    assert_eq!(renamed_metadata.title, "Renamed session");
    assert_eq!(renamed_metadata.workspace_id, created_metadata.workspace_id);
    assert_eq!(
        renamed_metadata.last_activity_at,
        created_metadata.last_activity_at
    );

    let active_metadata = post_message_and_load_metadata(context, &created.id).await;

    assert_eq!(active_metadata.status, "active");
    assert!(active_metadata.last_activity_at >= renamed_metadata.last_activity_at);

    let closed_metadata = close_session_and_load_metadata(context, &created.id).await;

    assert_eq!(closed_metadata.status, "closed");
    assert!(closed_metadata.closed_at.is_some());

    let deleted_metadata = delete_session_and_load_metadata(context, &created.id).await;

    assert_eq!(deleted_metadata.status, "deleted");
    assert!(deleted_metadata.deleted_at.is_some());
    let snapshot_error = context
        .store
        .session_snapshot(&context.live_owner_id, &created.id)
        .await
        .expect_err("deleted sessions should be removed from the live store");
    assert_eq!(snapshot_error, SessionStoreError::NotFound);
}

async fn create_owned_workspace_for_principal(
    state: &AppState,
    principal: Extension<AuthenticatedPrincipal>,
    name: &str,
) -> crate::contract_workspaces::WorkspaceDetail {
    create_workspace(
        State(state.clone()),
        principal,
        Json(CreateWorkspaceRequest {
            name: name.to_string(),
            upstream_url: None,
            default_ref: None,
            credential_reference_id: None,
        }),
    )
    .await
    .expect("workspace creation should succeed")
    .1
    .0
    .workspace
}

async fn create_persisted_session(
    context: &MetadataTestContext,
) -> (SessionSnapshot, SessionMetadataRecord) {
    let workspace = create_owned_workspace_for_principal(
        &context.state,
        context.principal.clone(),
        "Metadata Workspace",
    )
    .await;
    let session = create_workspace_session(
        State(context.state.clone()),
        Path(workspace.workspace_id),
        context.principal.clone(),
    )
    .await
    .expect("session creation should succeed")
    .1
    .0
    .session;
    let metadata = load_session_metadata_or_panic(
        context.workspace_repository.as_ref(),
        &context.user.user_id,
        &session.id,
        "created",
    )
    .await;

    (session, metadata)
}

async fn rename_persisted_session(
    context: &MetadataTestContext,
    session_id: &str,
) -> (SessionSnapshot, SessionMetadataRecord) {
    let session = rename_session(
        State(context.state.clone()),
        Path(session_id.to_string()),
        context.principal.clone(),
        Json(RenameSessionRequest {
            title: "Renamed session".to_string(),
        }),
    )
    .await
    .expect("session rename should succeed")
    .0
    .session;
    let metadata = load_session_metadata_or_panic(
        context.workspace_repository.as_ref(),
        &context.user.user_id,
        session_id,
        "renamed",
    )
    .await;

    (session, metadata)
}

async fn post_message_and_load_metadata(
    context: &MetadataTestContext,
    session_id: &str,
) -> SessionMetadataRecord {
    let _ = post_message(
        State(context.state.clone()),
        Path(session_id.to_string()),
        context.principal.clone(),
        Json(PromptRequest {
            text: "hello metadata".to_string(),
        }),
    )
    .await
    .expect("prompt submission should succeed");

    load_session_metadata_or_panic(
        context.workspace_repository.as_ref(),
        &context.user.user_id,
        session_id,
        "active",
    )
    .await
}

async fn close_session_and_load_metadata(
    context: &MetadataTestContext,
    session_id: &str,
) -> SessionMetadataRecord {
    let _ = close_session(
        State(context.state.clone()),
        Path(session_id.to_string()),
        context.principal.clone(),
    )
    .await
    .expect("session close should succeed");

    load_session_metadata_or_panic(
        context.workspace_repository.as_ref(),
        &context.user.user_id,
        session_id,
        "closed",
    )
    .await
}

async fn delete_session_and_load_metadata(
    context: &MetadataTestContext,
    session_id: &str,
) -> SessionMetadataRecord {
    let _ = delete_session(
        State(context.state.clone()),
        Path(session_id.to_string()),
        context.principal.clone(),
    )
    .await
    .expect("session deletion should succeed");

    load_session_metadata_or_panic(
        context.workspace_repository.as_ref(),
        &context.user.user_id,
        session_id,
        "deleted",
    )
    .await
}

fn bearer_headers(owner: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::AUTHORIZATION,
        format!("Bearer {owner}")
            .parse()
            .expect("authorization should parse"),
    );
    headers
}

fn bearer_principal(owner: &str) -> Extension<AuthenticatedPrincipal> {
    Extension(authorize_request(&bearer_headers(owner), false).expect("headers should authorize"))
}

fn browser_cookie_headers(response: &Response) -> HeaderMap {
    let mut headers = HeaderMap::new();
    let cookie_header = response
        .headers()
        .get_all(SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .filter_map(|cookie| cookie.split(';').next())
        .collect::<Vec<_>>()
        .join("; ");
    headers.insert(COOKIE, HeaderValue::from_str(&cookie_header).unwrap());
    headers
}

#[derive(Clone)]
struct BrowserAuthContext {
    headers: HeaderMap,
    principal: AuthenticatedPrincipal,
}

impl BrowserAuthContext {
    async fn spawn() -> Self {
        let shell = app_entrypoint(HeaderMap::new()).await;
        let headers = browser_cookie_headers(&shell);
        let body = to_bytes(shell.into_body(), usize::MAX)
            .await
            .expect("entrypoint body should be readable");
        let body = String::from_utf8(body.to_vec()).expect("entrypoint body should be utf-8");
        let csrf_token = extract_meta_content(&body, "acp-csrf-token");

        Self {
            principal: authorize_browser_headers(&headers, &csrf_token),
            headers,
        }
    }
}

fn auth_test_state() -> AppState {
    AppState::with_workspace_repository(
        Arc::new(SessionStore::new(4)),
        new_ephemeral_workspace_repository(),
        Arc::new(StaticReplyProvider {
            reply: "test reply".to_string(),
        }),
    )
}

fn tracking_auth_test_state() -> (AppState, StdArc<Mutex<Vec<String>>>) {
    let forgotten_sessions = StdArc::new(Mutex::new(Vec::new()));
    let state = AppState::with_workspace_repository(
        Arc::new(SessionStore::new(4)),
        new_ephemeral_workspace_repository(),
        Arc::new(TrackingReplyProvider {
            forgotten_sessions: forgotten_sessions.clone(),
        }),
    );
    (state, forgotten_sessions)
}

fn authorize_browser_headers(headers: &HeaderMap, csrf_token: &str) -> AuthenticatedPrincipal {
    let mut write_headers = headers.clone();
    write_headers.insert("x-csrf-token", HeaderValue::from_str(csrf_token).unwrap());
    authorize_request(&write_headers, true).expect("browser headers should authorize")
}

async fn bootstrap_admin_account(state: &AppState, browser: &BrowserAuthContext) {
    let _registered = bootstrap_register(
        State(state.clone()),
        Extension(browser.principal.clone()),
        Json(BootstrapRegistrationRequest {
            username: "admin".to_string(),
            password: "password123".to_string(),
        }),
    )
    .await
    .expect("bootstrap registration should succeed");
}

async fn create_member_account(
    state: &AppState,
    admin_browser: &BrowserAuthContext,
    username: &str,
    password: &str,
) -> LocalAccount {
    create_account(
        State(state.clone()),
        Extension(admin_browser.principal.clone()),
        Json(CreateAccountRequest {
            username: username.to_string(),
            password: password.to_string(),
            is_admin: false,
        }),
    )
    .await
    .expect("member account creation should succeed")
    .1
    .0
    .account
}

async fn sign_in_browser_account(
    state: &AppState,
    browser: &BrowserAuthContext,
    username: &str,
    password: &str,
) -> LocalAccount {
    sign_in(
        State(state.clone()),
        Extension(browser.principal.clone()),
        Json(SignInRequest {
            username: username.to_string(),
            password: password.to_string(),
        }),
    )
    .await
    .expect("sign-in should succeed")
    .0
    .account
}

fn extract_meta_content(document: &str, name: &str) -> String {
    let name_needle = format!(r#"name="{name}""#);
    let tag = document
        .lines()
        .find(|line| line.contains("<meta ") && line.contains(&name_needle))
        .expect("meta tag should exist")
        .trim();
    let content_start = tag.find(r#"content=""#).unwrap() + r#"content=""#.len();
    let content_end = tag[content_start..].find('"').unwrap() + content_start;
    tag[content_start..content_end].to_string()
}

async fn materialized_user_for_headers(
    workspace_store: &SqliteWorkspaceRepository,
    headers: &HeaderMap,
) -> UserRecord {
    let principal = authorize_request(headers, true).expect("headers should authorize");
    workspace_store
        .materialize_user(&principal)
        .await
        .expect("principal materialization should be stable")
}

async fn load_session_metadata_or_panic(
    workspace_store: &SqliteWorkspaceRepository,
    user_id: &str,
    session_id: &str,
    stage: &str,
) -> SessionMetadataRecord {
    workspace_store
        .load_session_metadata(user_id, session_id)
        .await
        .unwrap_or_else(|_| panic!("{stage} session metadata should load"))
        .unwrap_or_else(|| panic!("{stage} session metadata should exist"))
}

fn failing_workspace_state(store: Arc<SessionStore>) -> AppState {
    AppState::with_workspace_repository(
        store,
        Arc::new(FailingWorkspaceStore::new("metadata unavailable")),
        Arc::new(TrackingReplyProvider {
            forgotten_sessions: StdArc::new(Mutex::new(Vec::new())),
        }),
    )
}

fn sample_user_record() -> UserRecord {
    let now = chrono::Utc::now();
    UserRecord {
        user_id: "u_test".to_string(),
        principal_kind: "bearer".to_string(),
        principal_subject: "durable-subject".to_string(),
        username: Some("admin".to_string()),
        password_hash: None,
        is_admin: true,
        created_at: now,
        last_seen_at: now,
        deleted_at: None,
    }
}

fn sample_snapshot(session_id: &str) -> SessionSnapshot {
    SessionSnapshot {
        id: session_id.to_string(),
        workspace_id: "w_test".to_string(),
        title: "Test session".to_string(),
        status: SessionStatus::Active,
        latest_sequence: 0,
        messages: Vec::new(),
        pending_permissions: Vec::new(),
    }
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
struct FailingWorkspaceStore {
    error: WorkspaceStoreError,
}

impl FailingWorkspaceStore {
    fn new(message: &str) -> Self {
        Self {
            error: WorkspaceStoreError::Database(message.to_string()),
        }
    }
}

#[derive(Debug)]
struct RollbackFailingMetadataWorkspaceStore {
    store: Arc<SessionStore>,
    live_owner: String,
    user: UserRecord,
    error: WorkspaceStoreError,
    discard_before_fail: bool,
}

impl RollbackFailingMetadataWorkspaceStore {
    fn new(
        store: Arc<SessionStore>,
        live_owner: &str,
        message: &str,
        discard_before_fail: bool,
    ) -> Self {
        Self {
            store,
            live_owner: live_owner.to_string(),
            user: sample_user_record(),
            error: WorkspaceStoreError::Database(message.to_string()),
            discard_before_fail,
        }
    }
}

#[async_trait]
impl WorkspaceRepository for FailingWorkspaceStore {
    async fn materialize_user(
        &self,
        _principal: &AuthenticatedPrincipal,
    ) -> Result<UserRecord, WorkspaceStoreError> {
        Err(self.error.clone())
    }

    async fn bootstrap_workspace(
        &self,
        _owner_user_id: &str,
    ) -> Result<WorkspaceRecord, WorkspaceStoreError> {
        Err(self.error.clone())
    }

    async fn list_workspaces(
        &self,
        _owner_user_id: &str,
    ) -> Result<Vec<WorkspaceRecord>, WorkspaceStoreError> {
        Err(self.error.clone())
    }

    async fn load_workspace(
        &self,
        _owner_user_id: &str,
        _workspace_id: &str,
    ) -> Result<Option<WorkspaceRecord>, WorkspaceStoreError> {
        Err(self.error.clone())
    }

    async fn create_workspace(
        &self,
        _owner_user_id: &str,
        _request: &CreateWorkspaceRequest,
    ) -> Result<WorkspaceRecord, WorkspaceStoreError> {
        Err(self.error.clone())
    }

    async fn update_workspace(
        &self,
        _owner_user_id: &str,
        _workspace_id: &str,
        _request: &UpdateWorkspaceRequest,
    ) -> Result<WorkspaceRecord, WorkspaceStoreError> {
        Err(self.error.clone())
    }

    async fn delete_workspace(
        &self,
        _owner_user_id: &str,
        _workspace_id: &str,
    ) -> Result<(), WorkspaceStoreError> {
        Err(self.error.clone())
    }

    async fn list_workspace_sessions(
        &self,
        _owner_user_id: &str,
        _workspace_id: &str,
    ) -> Result<Vec<crate::contract_sessions::SessionListItem>, WorkspaceStoreError> {
        Err(self.error.clone())
    }

    async fn save_session_metadata(
        &self,
        _record: &SessionMetadataRecord,
    ) -> Result<(), WorkspaceStoreError> {
        Err(self.error.clone())
    }

    async fn persist_session_snapshot(
        &self,
        _owner_user_id: &str,
        _snapshot: &SessionSnapshot,
        _touch_activity: bool,
        _status_override: Option<&str>,
    ) -> Result<(), WorkspaceStoreError> {
        Err(self.error.clone())
    }

    async fn load_session_metadata(
        &self,
        _owner_user_id: &str,
        _session_id: &str,
    ) -> Result<Option<SessionMetadataRecord>, WorkspaceStoreError> {
        Err(self.error.clone())
    }

    async fn load_session_snapshot(
        &self,
        _owner_user_id: &str,
        _session_id: &str,
    ) -> Result<Option<DurableSessionSnapshotRecord>, WorkspaceStoreError> {
        Err(self.error.clone())
    }

    async fn auth_status(
        &self,
        _browser_session_id: Option<&str>,
    ) -> Result<(bool, Option<UserRecord>), WorkspaceStoreError> {
        Err(self.error.clone())
    }

    async fn authenticate_browser_session(
        &self,
        _browser_session_id: &str,
    ) -> Result<Option<UserRecord>, WorkspaceStoreError> {
        Err(self.error.clone())
    }

    async fn bootstrap_local_account(
        &self,
        _browser_session_id: &str,
        _username: &str,
        _password: &str,
    ) -> Result<LocalAccount, WorkspaceStoreError> {
        Err(self.error.clone())
    }

    async fn sign_in_local_account(
        &self,
        _browser_session_id: &str,
        _username: &str,
        _password: &str,
    ) -> Result<LocalAccount, WorkspaceStoreError> {
        Err(self.error.clone())
    }

    async fn sign_out_browser_session(
        &self,
        _browser_session_id: &str,
    ) -> Result<(), WorkspaceStoreError> {
        Err(self.error.clone())
    }

    async fn list_local_accounts(&self) -> Result<Vec<LocalAccount>, WorkspaceStoreError> {
        Err(self.error.clone())
    }

    async fn create_local_account(
        &self,
        _username: &str,
        _password: &str,
        _is_admin: bool,
    ) -> Result<LocalAccount, WorkspaceStoreError> {
        Err(self.error.clone())
    }

    async fn update_local_account(
        &self,
        _target_user_id: &str,
        _current_user_id: &str,
        _password: Option<&str>,
        _is_admin: Option<bool>,
    ) -> Result<LocalAccount, WorkspaceStoreError> {
        Err(self.error.clone())
    }

    async fn delete_local_account(
        &self,
        _target_user_id: &str,
        _current_user_id: &str,
    ) -> Result<Vec<String>, WorkspaceStoreError> {
        Err(self.error.clone())
    }
}

#[async_trait]
impl WorkspaceRepository for RollbackFailingMetadataWorkspaceStore {
    async fn materialize_user(
        &self,
        _principal: &AuthenticatedPrincipal,
    ) -> Result<UserRecord, WorkspaceStoreError> {
        Ok(self.user.clone())
    }

    async fn bootstrap_workspace(
        &self,
        owner_user_id: &str,
    ) -> Result<WorkspaceRecord, WorkspaceStoreError> {
        Ok(WorkspaceRecord {
            workspace_id: "w_test".to_string(),
            owner_user_id: owner_user_id.to_string(),
            name: "Workspace A".to_string(),
            upstream_url: None,
            default_ref: None,
            credential_reference_id: None,
            bootstrap_kind: None,
            status: "active".to_string(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            deleted_at: None,
        })
    }

    async fn list_workspaces(
        &self,
        owner_user_id: &str,
    ) -> Result<Vec<WorkspaceRecord>, WorkspaceStoreError> {
        Ok(vec![self.bootstrap_workspace(owner_user_id).await?])
    }

    async fn load_workspace(
        &self,
        owner_user_id: &str,
        workspace_id: &str,
    ) -> Result<Option<WorkspaceRecord>, WorkspaceStoreError> {
        let workspace = self.bootstrap_workspace(owner_user_id).await?;
        Ok((workspace.workspace_id == workspace_id).then_some(workspace))
    }

    async fn create_workspace(
        &self,
        owner_user_id: &str,
        request: &CreateWorkspaceRequest,
    ) -> Result<WorkspaceRecord, WorkspaceStoreError> {
        Ok(WorkspaceRecord {
            workspace_id: "w_created".to_string(),
            owner_user_id: owner_user_id.to_string(),
            name: request.name.clone(),
            upstream_url: request.upstream_url.clone(),
            default_ref: request.default_ref.clone(),
            credential_reference_id: request.credential_reference_id.clone(),
            bootstrap_kind: None,
            status: "active".to_string(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            deleted_at: None,
        })
    }

    async fn update_workspace(
        &self,
        owner_user_id: &str,
        workspace_id: &str,
        request: &UpdateWorkspaceRequest,
    ) -> Result<WorkspaceRecord, WorkspaceStoreError> {
        Ok(WorkspaceRecord {
            workspace_id: workspace_id.to_string(),
            owner_user_id: owner_user_id.to_string(),
            name: request
                .name
                .clone()
                .unwrap_or_else(|| "updated".to_string()),
            upstream_url: None,
            default_ref: request.default_ref.clone(),
            credential_reference_id: None,
            bootstrap_kind: None,
            status: "active".to_string(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            deleted_at: None,
        })
    }

    async fn delete_workspace(
        &self,
        _owner_user_id: &str,
        _workspace_id: &str,
    ) -> Result<(), WorkspaceStoreError> {
        Ok(())
    }

    async fn list_workspace_sessions(
        &self,
        _owner_user_id: &str,
        workspace_id: &str,
    ) -> Result<Vec<crate::contract_sessions::SessionListItem>, WorkspaceStoreError> {
        Ok(vec![crate::contract_sessions::SessionListItem {
            id: "s_test".to_string(),
            workspace_id: workspace_id.to_string(),
            title: "Session".to_string(),
            status: SessionStatus::Active,
            last_activity_at: chrono::Utc::now(),
        }])
    }

    async fn save_session_metadata(
        &self,
        _record: &SessionMetadataRecord,
    ) -> Result<(), WorkspaceStoreError> {
        Ok(())
    }

    async fn persist_session_snapshot(
        &self,
        _owner_user_id: &str,
        snapshot: &SessionSnapshot,
        _touch_activity: bool,
        _status_override: Option<&str>,
    ) -> Result<(), WorkspaceStoreError> {
        if self.discard_before_fail {
            let _ = self
                .store
                .discard_session(&self.live_owner, &snapshot.id)
                .await;
        }
        Err(self.error.clone())
    }

    async fn load_session_metadata(
        &self,
        _owner_user_id: &str,
        _session_id: &str,
    ) -> Result<Option<SessionMetadataRecord>, WorkspaceStoreError> {
        Ok(None)
    }

    async fn load_session_snapshot(
        &self,
        _owner_user_id: &str,
        _session_id: &str,
    ) -> Result<Option<DurableSessionSnapshotRecord>, WorkspaceStoreError> {
        Ok(None)
    }

    async fn auth_status(
        &self,
        _browser_session_id: Option<&str>,
    ) -> Result<(bool, Option<UserRecord>), WorkspaceStoreError> {
        Ok((false, Some(self.user.clone())))
    }

    async fn authenticate_browser_session(
        &self,
        _browser_session_id: &str,
    ) -> Result<Option<UserRecord>, WorkspaceStoreError> {
        Ok(Some(self.user.clone()))
    }

    async fn bootstrap_local_account(
        &self,
        _browser_session_id: &str,
        _username: &str,
        _password: &str,
    ) -> Result<LocalAccount, WorkspaceStoreError> {
        Ok(LocalAccount {
            user_id: self.user.user_id.clone(),
            username: self
                .user
                .username
                .clone()
                .unwrap_or_else(|| "admin".to_string()),
            is_admin: self.user.is_admin,
            created_at: self.user.created_at,
        })
    }

    async fn sign_in_local_account(
        &self,
        _browser_session_id: &str,
        _username: &str,
        _password: &str,
    ) -> Result<LocalAccount, WorkspaceStoreError> {
        self.bootstrap_local_account("", "", "").await
    }

    async fn sign_out_browser_session(
        &self,
        _browser_session_id: &str,
    ) -> Result<(), WorkspaceStoreError> {
        Ok(())
    }

    async fn list_local_accounts(&self) -> Result<Vec<LocalAccount>, WorkspaceStoreError> {
        Ok(vec![LocalAccount {
            user_id: self.user.user_id.clone(),
            username: self
                .user
                .username
                .clone()
                .unwrap_or_else(|| "admin".to_string()),
            is_admin: self.user.is_admin,
            created_at: self.user.created_at,
        }])
    }

    async fn create_local_account(
        &self,
        _username: &str,
        _password: &str,
        _is_admin: bool,
    ) -> Result<LocalAccount, WorkspaceStoreError> {
        self.bootstrap_local_account("", "", "").await
    }

    async fn update_local_account(
        &self,
        _target_user_id: &str,
        _current_user_id: &str,
        _password: Option<&str>,
        _is_admin: Option<bool>,
    ) -> Result<LocalAccount, WorkspaceStoreError> {
        self.bootstrap_local_account("", "", "").await
    }

    async fn delete_local_account(
        &self,
        _target_user_id: &str,
        _current_user_id: &str,
    ) -> Result<Vec<String>, WorkspaceStoreError> {
        Ok(Vec::new())
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
