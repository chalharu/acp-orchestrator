use crate::contract_sessions::SessionSnapshot;
use chrono::{DateTime, Utc};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserRecord {
    pub user_id: String,
    pub principal_kind: String,
    pub principal_subject: String,
    pub username: Option<String>,
    pub password_hash: Option<String>,
    pub is_admin: bool,
    pub created_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRecord {
    pub workspace_id: String,
    pub owner_user_id: String,
    pub name: String,
    pub upstream_url: Option<String>,
    pub default_ref: Option<String>,
    pub credential_reference_id: Option<String>,
    pub bootstrap_kind: Option<String>,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionMetadataRecord {
    pub session_id: String,
    pub workspace_id: String,
    pub owner_user_id: String,
    pub title: String,
    pub status: String,
    pub checkout_relpath: Option<String>,
    pub checkout_ref: Option<String>,
    pub checkout_commit_sha: Option<String>,
    pub failure_reason: Option<String>,
    pub detach_deadline_at: Option<DateTime<Utc>>,
    pub restartable_deadline_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub last_activity_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurableSessionSnapshotRecord {
    pub session: SessionSnapshot,
    pub last_activity_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceStoreError {
    Io(String),
    Database(String),
    Unauthorized(String),
    NotFound(String),
    Conflict(String),
    Validation(String),
}

impl WorkspaceStoreError {
    pub fn message(&self) -> &str {
        match self {
            Self::Io(message)
            | Self::Database(message)
            | Self::Unauthorized(message)
            | Self::NotFound(message)
            | Self::Conflict(message)
            | Self::Validation(message) => message,
        }
    }
}

impl fmt::Display for WorkspaceStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for WorkspaceStoreError {}
