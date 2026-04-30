use super::super::super::{AppError, AppState, OwnerContext};
use super::super::{
    SessionStartupContext, bind_session_launch_metadata, cleanup_checkout_path_best_effort,
    load_checkout_cleanup_path_best_effort, map_agent_profile_error, map_checkout_error,
    persist_failed_session_lifecycle, persist_provisioning_session_lifecycle,
    session_startup_options,
};
use crate::{
    agent_profiles::AgentProfileStoreError,
    auth::{AuthenticatedPrincipal, AuthenticatedPrincipalKind},
    contract_accounts::LocalAccount,
    contract_sessions::{AgentProfileMode, SessionStatus, UpsertAgentProfileRequest},
    mock_client::{BindLaunchMetadataFuture, ReplyFuture, ReplyProvider, ReplyResult},
    sessions::{SessionStore, SessionStoreError},
    workspace_checkout::{
        PreparedWorkspaceCheckout, WorkspaceCheckoutLayout, WorkspaceCheckoutManager,
    },
    workspace_records::{
        DurableSessionSnapshotRecord, SessionMetadataRecord, UserRecord, WorkspaceRecord,
        WorkspaceStoreError,
    },
    workspace_repository::{NewWorkspace, WorkspaceRepository, WorkspaceUpdatePatch},
};
use async_trait::async_trait;
use chrono::Utc;
use futures_util::FutureExt;
use std::{future::Future, panic::AssertUnwindSafe, path::PathBuf, sync::Arc};

#[derive(Debug)]
struct NoopReplyProvider;

impl ReplyProvider for NoopReplyProvider {
    fn request_reply<'a>(&'a self, _turn: crate::sessions::TurnHandle) -> ReplyFuture<'a> {
        Box::pin(async { Ok(ReplyResult::NoOutput) })
    }
}

#[derive(Debug)]
struct MetadataFailingReplyProvider;

impl ReplyProvider for MetadataFailingReplyProvider {
    fn request_reply<'a>(&'a self, _turn: crate::sessions::TurnHandle) -> ReplyFuture<'a> {
        Box::pin(async { Ok(ReplyResult::NoOutput) })
    }

    fn bind_session_launch_metadata<'a>(
        &'a self,
        _session_id: &'a str,
        _metadata: crate::agent_runtime::AgentLaunchMetadata,
    ) -> BindLaunchMetadataFuture<'a> {
        Box::pin(async { Err("metadata bind failed".to_string()) })
    }
}

#[derive(Debug)]
struct StubWorkspaceRepository {
    metadata: Option<SessionMetadataRecord>,
    load_error: Option<WorkspaceStoreError>,
    save_error: Option<WorkspaceStoreError>,
}

#[async_trait]
impl WorkspaceRepository for StubWorkspaceRepository {
    async fn materialize_user(
        &self,
        _principal: &AuthenticatedPrincipal,
    ) -> Result<UserRecord, WorkspaceStoreError> {
        unimplemented!("not used in session_service unit tests")
    }

    async fn bootstrap_workspace(
        &self,
        _owner_user_id: &str,
    ) -> Result<WorkspaceRecord, WorkspaceStoreError> {
        unimplemented!("not used in session_service unit tests")
    }

    async fn list_workspaces(
        &self,
        _owner_user_id: &str,
    ) -> Result<Vec<WorkspaceRecord>, WorkspaceStoreError> {
        unimplemented!("not used in session_service unit tests")
    }

    async fn load_workspace(
        &self,
        _owner_user_id: &str,
        _workspace_id: &str,
    ) -> Result<Option<WorkspaceRecord>, WorkspaceStoreError> {
        unimplemented!("not used in session_service unit tests")
    }

    async fn create_workspace(
        &self,
        _owner_user_id: &str,
        _workspace: &NewWorkspace,
    ) -> Result<WorkspaceRecord, WorkspaceStoreError> {
        unimplemented!("not used in session_service unit tests")
    }

    async fn update_workspace(
        &self,
        _owner_user_id: &str,
        _workspace_id: &str,
        _update: &WorkspaceUpdatePatch,
    ) -> Result<WorkspaceRecord, WorkspaceStoreError> {
        unimplemented!("not used in session_service unit tests")
    }

    async fn delete_workspace(
        &self,
        _owner_user_id: &str,
        _workspace_id: &str,
    ) -> Result<(), WorkspaceStoreError> {
        unimplemented!("not used in session_service unit tests")
    }

    async fn list_workspace_sessions(
        &self,
        _owner_user_id: &str,
        _workspace_id: &str,
    ) -> Result<Vec<crate::contract_sessions::SessionListItem>, WorkspaceStoreError> {
        unimplemented!("not used in session_service unit tests")
    }

    async fn save_session_metadata(
        &self,
        _record: &SessionMetadataRecord,
    ) -> Result<(), WorkspaceStoreError> {
        match &self.save_error {
            Some(error) => Err(error.clone()),
            None => Ok(()),
        }
    }

    async fn persist_session_snapshot(
        &self,
        _owner_user_id: &str,
        _snapshot: &crate::contract_sessions::SessionSnapshot,
        _touch_activity: bool,
        _status_override: Option<&str>,
    ) -> Result<(), WorkspaceStoreError> {
        Ok(())
    }

    async fn load_session_metadata(
        &self,
        _owner_user_id: &str,
        _session_id: &str,
    ) -> Result<Option<SessionMetadataRecord>, WorkspaceStoreError> {
        match &self.load_error {
            Some(error) => Err(error.clone()),
            None => Ok(self.metadata.clone()),
        }
    }

    async fn load_session_snapshot(
        &self,
        _owner_user_id: &str,
        _session_id: &str,
    ) -> Result<Option<DurableSessionSnapshotRecord>, WorkspaceStoreError> {
        unimplemented!("not used in session_service unit tests")
    }

    async fn auth_status(
        &self,
        _browser_session_id: Option<&str>,
    ) -> Result<(bool, Option<UserRecord>), WorkspaceStoreError> {
        unimplemented!("not used in session_service unit tests")
    }

    async fn authenticate_browser_session(
        &self,
        _browser_session_id: &str,
    ) -> Result<Option<UserRecord>, WorkspaceStoreError> {
        unimplemented!("not used in session_service unit tests")
    }

    async fn bootstrap_local_account(
        &self,
        _browser_session_id: &str,
        _username: &str,
        _password: &str,
    ) -> Result<LocalAccount, WorkspaceStoreError> {
        unimplemented!("not used in session_service unit tests")
    }

    async fn sign_in_local_account(
        &self,
        _browser_session_id: &str,
        _username: &str,
        _password: &str,
    ) -> Result<LocalAccount, WorkspaceStoreError> {
        unimplemented!("not used in session_service unit tests")
    }

    async fn sign_out_browser_session(
        &self,
        _browser_session_id: &str,
    ) -> Result<(), WorkspaceStoreError> {
        unimplemented!("not used in session_service unit tests")
    }

    async fn list_local_accounts(&self) -> Result<Vec<LocalAccount>, WorkspaceStoreError> {
        unimplemented!("not used in session_service unit tests")
    }

    async fn create_local_account(
        &self,
        _username: &str,
        _password: &str,
        _is_admin: bool,
    ) -> Result<LocalAccount, WorkspaceStoreError> {
        unimplemented!("not used in session_service unit tests")
    }

    async fn update_local_account(
        &self,
        _target_user_id: &str,
        _current_user_id: &str,
        _password: Option<&str>,
        _is_admin: Option<bool>,
    ) -> Result<LocalAccount, WorkspaceStoreError> {
        unimplemented!("not used in session_service unit tests")
    }

    async fn delete_local_account(
        &self,
        _target_user_id: &str,
        _current_user_id: &str,
    ) -> Result<Vec<String>, WorkspaceStoreError> {
        unimplemented!("not used in session_service unit tests")
    }
}

#[derive(Debug)]
struct InvalidCheckoutManager;

#[async_trait]
impl WorkspaceCheckoutManager for InvalidCheckoutManager {
    async fn prepare_checkout(
        &self,
        _workspace: &WorkspaceRecord,
        _session_id: &str,
        _checkout_ref_override: Option<&str>,
    ) -> Result<PreparedWorkspaceCheckout, crate::workspace_checkout::WorkspaceCheckoutError> {
        unimplemented!("not used in session_service unit tests")
    }

    fn resolve_checkout_path(&self, _checkout_relpath: &str) -> Option<PathBuf> {
        None
    }

    fn checkout_relpath_for_session(&self, session_id: &str) -> Option<String> {
        Some(format!("session-checkouts/{session_id}"))
    }
}

fn sample_user() -> UserRecord {
    let now = Utc::now();
    UserRecord {
        user_id: "u_test".to_string(),
        principal_kind: "bearer".to_string(),
        principal_subject: "alice".to_string(),
        username: Some("alice".to_string()),
        password_hash: None,
        is_admin: true,
        created_at: now,
        last_seen_at: now,
        deleted_at: None,
    }
}

fn sample_workspace() -> WorkspaceRecord {
    let now = Utc::now();
    WorkspaceRecord {
        workspace_id: "w_test".to_string(),
        owner_user_id: "u_test".to_string(),
        name: "Workspace".to_string(),
        upstream_url: None,
        default_ref: None,
        credential_reference_id: None,
        bootstrap_kind: None,
        status: "active".to_string(),
        created_at: now,
        updated_at: now,
        deleted_at: None,
    }
}

fn sample_metadata(checkout_relpath: Option<&str>) -> SessionMetadataRecord {
    let now = Utc::now();
    SessionMetadataRecord {
        session_id: "s_test".to_string(),
        workspace_id: "w_test".to_string(),
        owner_user_id: "u_test".to_string(),
        title: "Session".to_string(),
        status: "active".to_string(),
        checkout_relpath: checkout_relpath.map(str::to_string),
        checkout_ref: None,
        checkout_commit_sha: None,
        agent_profile_id: None,
        failure_reason: None,
        detach_deadline_at: None,
        restartable_deadline_at: None,
        created_at: now,
        last_activity_at: now,
        closed_at: None,
        deleted_at: None,
    }
}

fn sample_snapshot(session_id: &str) -> crate::contract_sessions::SessionSnapshot {
    crate::contract_sessions::SessionSnapshot {
        id: session_id.to_string(),
        workspace_id: "w_test".to_string(),
        title: "Session".to_string(),
        status: SessionStatus::Active,
        latest_sequence: 0,
        messages: Vec::new(),
        pending_permissions: Vec::new(),
        active_turn: false,
    }
}

fn unique_checkout_fixture() -> (PathBuf, PathBuf) {
    let checkout_root = std::env::temp_dir().join(format!(
        "acp-session-checkout-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let working_dir = checkout_root.join("checkout");
    std::fs::create_dir_all(&working_dir).expect("checkout should be creatable");
    (checkout_root, working_dir)
}

fn prepared_checkout_for(working_dir: PathBuf) -> PreparedWorkspaceCheckout {
    PreparedWorkspaceCheckout {
        checkout_relpath: "session-checkouts/s_test".to_string(),
        checkout_ref: None,
        checkout_commit_sha: None,
        working_dir,
    }
}

fn metadata_bind_failure_state(store: Arc<SessionStore>) -> AppState {
    AppState::with_workspace_repository(
        store,
        Arc::new(StubWorkspaceRepository {
            metadata: None,
            load_error: None,
            save_error: None,
        }),
        Arc::new(MetadataFailingReplyProvider),
    )
}

fn sample_principal() -> AuthenticatedPrincipal {
    AuthenticatedPrincipal {
        id: "alice".to_string(),
        kind: AuthenticatedPrincipalKind::Bearer,
        subject: "alice".to_string(),
    }
}

fn sample_new_workspace() -> NewWorkspace {
    NewWorkspace {
        name: "Workspace".to_string(),
        upstream_url: None,
        default_ref: None,
        credential_reference_id: None,
    }
}

fn sample_workspace_update() -> WorkspaceUpdatePatch {
    WorkspaceUpdatePatch {
        name: Some("Updated".to_string()),
        default_ref: Some("refs/heads/main".to_string()),
    }
}

fn agent_profile_request(mode: AgentProfileMode) -> UpsertAgentProfileRequest {
    UpsertAgentProfileRequest {
        name: format!("{mode:?} ACP"),
        mode,
        command_argv: vec!["agent".to_string(), "acp".to_string()],
        env_allowlist: Vec::new(),
        timeout_seconds: 30,
        run_uid: 65_534,
        run_gid: 65_534,
    }
}

async fn sample_turn_handle() -> crate::sessions::TurnHandle {
    let store = SessionStore::new(4);
    let session = store
        .create_session("alice", "w_test")
        .await
        .expect("session creation should succeed");
    store
        .submit_prompt("alice", &session.id, "hello".to_string())
        .await
        .expect("prompt submission should succeed")
        .turn_handle()
}

#[test]
fn session_startup_options_use_profile_specific_checkout_layouts() {
    let state = metadata_bind_failure_state(Arc::new(SessionStore::new(4)));
    state
        .agent_profile_store
        .upsert_profile("host", agent_profile_request(AgentProfileMode::Host))
        .expect("host profile should save");
    state
        .agent_profile_store
        .upsert_profile("chroot", agent_profile_request(AgentProfileMode::Chroot))
        .expect("chroot profile should save");

    let host = session_startup_options(&state, Some("host")).expect("host startup options");
    let chroot = session_startup_options(&state, Some("chroot")).expect("chroot startup options");

    assert_eq!(host.layout, WorkspaceCheckoutLayout::Standard);
    assert_eq!(
        host.agent_config.expect("host config").mode,
        crate::agent_runtime::AgentLaunchMode::Host
    );
    assert_eq!(chroot.layout, WorkspaceCheckoutLayout::ChrootRuntime);
    assert_eq!(
        chroot.agent_config.expect("chroot config").mode,
        crate::agent_runtime::AgentLaunchMode::Chroot
    );
}

async fn assert_future_panics<F, T>(future: F)
where
    F: Future<Output = T>,
{
    let result = AssertUnwindSafe(future).catch_unwind().await;
    assert!(result.is_err(), "future should panic");
}

#[tokio::test]
async fn noop_reply_provider_returns_no_output() {
    let reply = NoopReplyProvider
        .request_reply(sample_turn_handle().await)
        .await
        .expect("noop reply providers should return successfully");

    assert_eq!(reply, ReplyResult::NoOutput);
}

#[tokio::test]
async fn stub_workspace_repository_workspace_methods_panic_when_unused() {
    let repo = StubWorkspaceRepository {
        metadata: None,
        load_error: None,
        save_error: None,
    };

    assert_future_panics(repo.materialize_user(&sample_principal())).await;
    assert_future_panics(repo.bootstrap_workspace("u_test")).await;
    assert_future_panics(repo.list_workspaces("u_test")).await;
    assert_future_panics(repo.load_workspace("u_test", "w_test")).await;
    assert_future_panics(repo.create_workspace("u_test", &sample_new_workspace())).await;
    assert_future_panics(repo.update_workspace("u_test", "w_test", &sample_workspace_update()))
        .await;
    assert_future_panics(repo.delete_workspace("u_test", "w_test")).await;
    assert_future_panics(repo.list_workspace_sessions("u_test", "w_test")).await;
}

#[tokio::test]
async fn stub_workspace_repository_session_methods_cover_remaining_branches() {
    let repo = StubWorkspaceRepository {
        metadata: None,
        load_error: None,
        save_error: None,
    };

    repo.persist_session_snapshot("u_test", &sample_snapshot("s_test"), true, None)
        .await
        .expect("stub snapshot persistence should succeed");
    assert_future_panics(repo.load_session_snapshot("u_test", "s_test")).await;
    assert_future_panics(repo.auth_status(None)).await;
    assert_future_panics(repo.authenticate_browser_session("browser")).await;
}

#[tokio::test]
async fn stub_workspace_repository_account_methods_panic_when_unused() {
    let repo = StubWorkspaceRepository {
        metadata: None,
        load_error: None,
        save_error: None,
    };

    assert_future_panics(repo.bootstrap_local_account("browser", "alice", "password")).await;
    assert_future_panics(repo.sign_in_local_account("browser", "alice", "password")).await;
    assert_future_panics(repo.sign_out_browser_session("browser")).await;
    assert_future_panics(repo.list_local_accounts()).await;
    assert_future_panics(repo.create_local_account("alice", "password", true)).await;
    assert_future_panics(repo.update_local_account(
        "u_test",
        "u_admin",
        Some("password"),
        Some(true),
    ))
    .await;
    assert_future_panics(repo.delete_local_account("u_test", "u_admin")).await;
}

#[tokio::test]
async fn invalid_checkout_manager_panics_when_prepare_is_called_directly() {
    assert_future_panics(InvalidCheckoutManager.prepare_checkout(
        &sample_workspace(),
        "s_test",
        None,
    ))
    .await;
}

#[tokio::test]
async fn provisioning_persistence_failures_roll_back_live_sessions() {
    let store = Arc::new(crate::sessions::SessionStore::new(4));
    let session = store
        .create_session("bearer:alice", "w_test")
        .await
        .expect("session creation should succeed");
    let state = AppState::with_workspace_repository(
        store.clone(),
        Arc::new(StubWorkspaceRepository {
            metadata: None,
            load_error: None,
            save_error: Some(WorkspaceStoreError::Database(
                "metadata unavailable".to_string(),
            )),
        }),
        Arc::new(NoopReplyProvider),
    );
    let owner = OwnerContext {
        user: sample_user(),
        live_owner_id: "bearer:alice".to_string(),
    };

    let error =
        persist_provisioning_session_lifecycle(&state, &owner, &sample_workspace(), &session, None)
            .await
            .expect_err("metadata failures should abort provisioning");

    assert!(matches!(error, AppError::Internal(message) if message == "internal server error"));
    assert_eq!(
        store
            .session_snapshot("bearer:alice", &session.id)
            .await
            .expect_err("failed provisioning should discard the session"),
        SessionStoreError::NotFound
    );
}

#[tokio::test]
async fn launch_metadata_binding_failures_clean_up_and_roll_back_live_sessions() {
    let store = Arc::new(crate::sessions::SessionStore::new(4));
    let live_owner_id = "bearer:alice";
    let session = store
        .create_session(live_owner_id, "w_test")
        .await
        .expect("session creation should succeed");
    let (checkout_root, working_dir) = unique_checkout_fixture();
    let checkout = prepared_checkout_for(working_dir.clone());
    let state = metadata_bind_failure_state(store.clone());
    let user = sample_user();
    let workspace = sample_workspace();
    let context = SessionStartupContext {
        state: &state,
        live_owner_id,
        user: &user,
        workspace: &workspace,
        session: &session,
    };

    let error = bind_session_launch_metadata(
        &context,
        &checkout,
        crate::agent_runtime::AgentLaunchMetadata {
            acp_address: Some("127.0.0.1:12345".to_string()),
        },
    )
    .await
    .expect_err("metadata bind failures should abort startup");

    assert!(matches!(error, AppError::Internal(message) if message == "metadata bind failed"));
    assert!(
        !working_dir.exists(),
        "checkout directory should be removed after bind failure"
    );
    assert_eq!(
        store
            .session_snapshot(live_owner_id, &session.id)
            .await
            .expect_err("failed metadata binding should discard the session"),
        SessionStoreError::NotFound
    );
    let _ = std::fs::remove_dir(&checkout_root);
}

#[tokio::test]
async fn failed_session_persistence_warnings_do_not_propagate() {
    let state = AppState::with_workspace_repository(
        Arc::new(crate::sessions::SessionStore::new(4)),
        Arc::new(StubWorkspaceRepository {
            metadata: None,
            load_error: None,
            save_error: Some(WorkspaceStoreError::Database(
                "metadata unavailable".to_string(),
            )),
        }),
        Arc::new(NoopReplyProvider),
    );

    persist_failed_session_lifecycle(
        &state,
        &sample_user(),
        &sample_workspace(),
        &sample_snapshot("s_failed"),
        None,
        "checkout failed",
    )
    .await;
}

#[tokio::test]
async fn checkout_cleanup_path_loading_handles_invalid_and_unreadable_metadata() {
    let invalid_path_state = AppState::with_workspace_repository_and_checkout_manager(
        Arc::new(crate::sessions::SessionStore::new(4)),
        Arc::new(StubWorkspaceRepository {
            metadata: Some(sample_metadata(Some("../escape"))),
            load_error: None,
            save_error: None,
        }),
        Arc::new(NoopReplyProvider),
        Arc::new(InvalidCheckoutManager),
    );
    let user = sample_user();

    assert_eq!(
        load_checkout_cleanup_path_best_effort(&invalid_path_state, &user, "s_test", "delete")
            .await,
        None
    );

    let load_error_state = AppState::with_workspace_repository(
        Arc::new(crate::sessions::SessionStore::new(4)),
        Arc::new(StubWorkspaceRepository {
            metadata: None,
            load_error: Some(WorkspaceStoreError::Database(
                "metadata unavailable".to_string(),
            )),
            save_error: None,
        }),
        Arc::new(NoopReplyProvider),
    );

    assert_eq!(
        load_checkout_cleanup_path_best_effort(&load_error_state, &user, "s_test", "delete").await,
        None
    );
}

#[tokio::test]
async fn checkout_cleanup_path_loading_handles_metadata_without_checkout_paths() {
    let state = AppState::with_workspace_repository(
        Arc::new(crate::sessions::SessionStore::new(4)),
        Arc::new(StubWorkspaceRepository {
            metadata: Some(sample_metadata(None)),
            load_error: None,
            save_error: None,
        }),
        Arc::new(NoopReplyProvider),
    );

    assert_eq!(
        load_checkout_cleanup_path_best_effort(&state, &sample_user(), "s_test", "delete").await,
        None
    );
}

#[test]
fn cleanup_checkout_path_best_effort_ignores_missing_paths_and_files() {
    let missing_path = std::env::temp_dir().join(format!(
        "nonexistent-session-checkout-{}",
        uuid::Uuid::new_v4().simple()
    ));
    cleanup_checkout_path_best_effort(&missing_path);

    let file_path = std::env::temp_dir().join(format!(
        "acp-session-cleanup-file-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(file_path.parent().expect("file path should have a parent"))
        .expect("parent dir should be creatable");
    std::fs::write(&file_path, "not a directory").expect("file path should be writable");

    cleanup_checkout_path_best_effort(&file_path);
    assert!(
        file_path.exists(),
        "file cleanups should fail without panicking"
    );
    let _ = std::fs::remove_file(file_path);
}

#[test]
fn checkout_errors_map_to_public_http_errors() {
    assert!(matches!(
        map_checkout_error(crate::workspace_checkout::WorkspaceCheckoutError::Validation(
            "bad ref".to_string()
        )),
        AppError::BadRequest(message) if message == "bad ref"
    ));
    assert!(matches!(
        map_checkout_error(crate::workspace_checkout::WorkspaceCheckoutError::Io(
            "disk failed".to_string()
        )),
        AppError::Internal(message) if message == "checkout preparation failed"
    ));
    assert!(matches!(
        map_checkout_error(crate::workspace_checkout::WorkspaceCheckoutError::Git(
            "git failed".to_string()
        )),
        AppError::Internal(message) if message == "checkout preparation failed"
    ));
}

#[test]
fn agent_profile_errors_map_to_public_http_errors() {
    assert!(matches!(
        map_agent_profile_error(AgentProfileStoreError::NotFound),
        AppError::BadRequest(message) if message == "agent profile not found"
    ));
    assert!(matches!(
        map_agent_profile_error(AgentProfileStoreError::Validation("bad profile".to_string())),
        AppError::BadRequest(message) if message == "bad profile"
    ));
    assert!(matches!(
        map_agent_profile_error(AgentProfileStoreError::Io("io failed".to_string())),
        AppError::Internal(message) if message == "io failed"
    ));
    assert!(matches!(
        map_agent_profile_error(AgentProfileStoreError::Json("json failed".to_string())),
        AppError::Internal(message) if message == "json failed"
    ));
}
