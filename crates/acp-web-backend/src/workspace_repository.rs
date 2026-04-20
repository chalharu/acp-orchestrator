use acp_contracts::SessionSnapshot;
use async_trait::async_trait;

use crate::{
    auth::AuthenticatedPrincipal,
    workspace_store::{SessionMetadataRecord, UserRecord, WorkspaceRecord, WorkspaceStoreError},
};

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
}
