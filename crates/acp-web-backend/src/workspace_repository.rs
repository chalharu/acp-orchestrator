use async_trait::async_trait;

use crate::auth::AuthenticatedPrincipal;
use crate::contract_accounts::LocalAccount;
use crate::contract_sessions::{SessionListItem, SessionSnapshot};
use crate::workspace_records::{
    DurableSessionSnapshotRecord, SessionMetadataRecord, UserRecord, WorkspaceRecord,
    WorkspaceStoreError,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewWorkspace {
    pub name: String,
    pub upstream_url: Option<String>,
    pub default_ref: Option<String>,
    pub credential_reference_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceUpdatePatch {
    pub name: Option<String>,
    pub default_ref: Option<String>,
}

#[async_trait]
pub trait WorkspaceRepository: Send + Sync {
    async fn materialize_user(
        &self,
        principal: &AuthenticatedPrincipal,
    ) -> Result<UserRecord, WorkspaceStoreError>;

    async fn bootstrap_workspace(
        &self,
        owner_user_id: &str,
    ) -> Result<WorkspaceRecord, WorkspaceStoreError>;

    async fn list_workspaces(
        &self,
        owner_user_id: &str,
    ) -> Result<Vec<WorkspaceRecord>, WorkspaceStoreError>;

    async fn load_workspace(
        &self,
        owner_user_id: &str,
        workspace_id: &str,
    ) -> Result<Option<WorkspaceRecord>, WorkspaceStoreError>;

    async fn create_workspace(
        &self,
        owner_user_id: &str,
        workspace: &NewWorkspace,
    ) -> Result<WorkspaceRecord, WorkspaceStoreError>;

    async fn update_workspace(
        &self,
        owner_user_id: &str,
        workspace_id: &str,
        update: &WorkspaceUpdatePatch,
    ) -> Result<WorkspaceRecord, WorkspaceStoreError>;

    async fn delete_workspace(
        &self,
        owner_user_id: &str,
        workspace_id: &str,
    ) -> Result<(), WorkspaceStoreError>;

    async fn list_workspace_sessions(
        &self,
        owner_user_id: &str,
        workspace_id: &str,
    ) -> Result<Vec<SessionListItem>, WorkspaceStoreError>;

    async fn save_session_metadata(
        &self,
        record: &SessionMetadataRecord,
    ) -> Result<(), WorkspaceStoreError>;

    async fn persist_session_snapshot(
        &self,
        owner_user_id: &str,
        snapshot: &SessionSnapshot,
        touch_activity: bool,
        status_override: Option<&str>,
    ) -> Result<(), WorkspaceStoreError>;

    async fn load_session_metadata(
        &self,
        owner_user_id: &str,
        session_id: &str,
    ) -> Result<Option<SessionMetadataRecord>, WorkspaceStoreError>;

    async fn load_session_snapshot(
        &self,
        owner_user_id: &str,
        session_id: &str,
    ) -> Result<Option<DurableSessionSnapshotRecord>, WorkspaceStoreError>;

    async fn auth_status(
        &self,
        browser_session_id: Option<&str>,
    ) -> Result<(bool, Option<UserRecord>), WorkspaceStoreError>;

    async fn authenticate_browser_session(
        &self,
        browser_session_id: &str,
    ) -> Result<Option<UserRecord>, WorkspaceStoreError>;

    async fn bootstrap_local_account(
        &self,
        browser_session_id: &str,
        username: &str,
        password: &str,
    ) -> Result<LocalAccount, WorkspaceStoreError>;

    async fn sign_in_local_account(
        &self,
        browser_session_id: &str,
        username: &str,
        password: &str,
    ) -> Result<LocalAccount, WorkspaceStoreError>;

    async fn sign_out_browser_session(
        &self,
        browser_session_id: &str,
    ) -> Result<(), WorkspaceStoreError>;

    async fn list_local_accounts(&self) -> Result<Vec<LocalAccount>, WorkspaceStoreError>;

    async fn create_local_account(
        &self,
        username: &str,
        password: &str,
        is_admin: bool,
    ) -> Result<LocalAccount, WorkspaceStoreError>;

    async fn update_local_account(
        &self,
        target_user_id: &str,
        current_user_id: &str,
        password: Option<&str>,
        is_admin: Option<bool>,
    ) -> Result<LocalAccount, WorkspaceStoreError>;

    async fn delete_local_account(
        &self,
        target_user_id: &str,
        current_user_id: &str,
    ) -> Result<Vec<String>, WorkspaceStoreError>;
}
