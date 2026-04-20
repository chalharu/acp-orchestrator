use std::{
    fmt,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use acp_contracts::{SessionSnapshot, SessionStatus};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use crate::auth::{AuthenticatedPrincipal, AuthenticatedPrincipalKind};

const BOOTSTRAP_WORKSPACE_KIND: &str = "legacy-session-routes";
const BOOTSTRAP_WORKSPACE_NAME: &str = "Default workspace";
const ACTIVE_WORKSPACE_STATUS: &str = "active";
const DURABLE_BEARER_PRINCIPAL_NAMESPACE: uuid::Uuid =
    uuid::Uuid::from_u128(0x402dbecf7ab1458ca5dc1548a2597cec);
const DURABLE_BROWSER_SESSION_PRINCIPAL_NAMESPACE: uuid::Uuid =
    uuid::Uuid::from_u128(0x6dbdfc9664a54920ac2086040c3a8232);
const WORKSPACE_STORE_SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS users (
    user_id TEXT PRIMARY KEY,
    principal_kind TEXT NOT NULL,
    principal_subject TEXT NOT NULL,
    created_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL,
    UNIQUE(principal_kind, principal_subject)
);

CREATE TABLE IF NOT EXISTS workspaces (
    workspace_id TEXT PRIMARY KEY,
    owner_user_id TEXT NOT NULL,
    name TEXT NOT NULL,
    upstream_url TEXT,
    default_ref TEXT,
    credential_reference_id TEXT,
    status TEXT NOT NULL,
    bootstrap_kind TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    FOREIGN KEY (owner_user_id) REFERENCES users(user_id)
);

CREATE UNIQUE INDEX IF NOT EXISTS workspaces_owner_bootstrap_kind_idx
    ON workspaces(owner_user_id, bootstrap_kind)
    WHERE bootstrap_kind IS NOT NULL;

CREATE TABLE IF NOT EXISTS sessions (
    session_id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL,
    owner_user_id TEXT NOT NULL,
    title TEXT NOT NULL,
    status TEXT NOT NULL,
    checkout_relpath TEXT,
    checkout_ref TEXT,
    checkout_commit_sha TEXT,
    failure_reason TEXT,
    detach_deadline_at TEXT,
    restartable_deadline_at TEXT,
    created_at TEXT NOT NULL,
    last_activity_at TEXT NOT NULL,
    closed_at TEXT,
    deleted_at TEXT,
    FOREIGN KEY (workspace_id) REFERENCES workspaces(workspace_id),
    FOREIGN KEY (owner_user_id) REFERENCES users(user_id)
);

CREATE INDEX IF NOT EXISTS sessions_owner_user_id_idx
    ON sessions(owner_user_id);

CREATE INDEX IF NOT EXISTS sessions_workspace_id_idx
    ON sessions(workspace_id);
"#;
const LOAD_BOOTSTRAP_WORKSPACE_SQL: &str = r#"
SELECT workspace_id, owner_user_id, name, status, created_at, updated_at, deleted_at
FROM workspaces
WHERE owner_user_id = ?1 AND bootstrap_kind = ?2
"#;
const LOAD_SESSION_METADATA_SQL: &str = r#"
SELECT
    session_id,
    workspace_id,
    owner_user_id,
    title,
    status,
    checkout_relpath,
    checkout_ref,
    checkout_commit_sha,
    failure_reason,
    detach_deadline_at,
    restartable_deadline_at,
    created_at,
    last_activity_at,
    closed_at,
    deleted_at
FROM sessions
WHERE owner_user_id = ?1 AND session_id = ?2
"#;
const INSERT_BOOTSTRAP_WORKSPACE_SQL: &str = r#"
INSERT INTO workspaces (
    workspace_id,
    owner_user_id,
    name,
    upstream_url,
    default_ref,
    credential_reference_id,
    status,
    bootstrap_kind,
    created_at,
    updated_at,
    deleted_at
) VALUES (?1, ?2, ?3, NULL, NULL, NULL, ?4, ?5, ?6, ?7, NULL)
"#;
const UPSERT_SESSION_METADATA_SQL: &str = r#"
INSERT INTO sessions (
    session_id,
    workspace_id,
    owner_user_id,
    title,
    status,
    checkout_relpath,
    checkout_ref,
    checkout_commit_sha,
    failure_reason,
    detach_deadline_at,
    restartable_deadline_at,
    created_at,
    last_activity_at,
    closed_at,
    deleted_at
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
ON CONFLICT(session_id) DO UPDATE SET
    workspace_id = excluded.workspace_id,
    owner_user_id = excluded.owner_user_id,
    title = excluded.title,
    status = excluded.status,
    checkout_relpath = excluded.checkout_relpath,
    checkout_ref = excluded.checkout_ref,
    checkout_commit_sha = excluded.checkout_commit_sha,
    failure_reason = excluded.failure_reason,
    detach_deadline_at = excluded.detach_deadline_at,
    restartable_deadline_at = excluded.restartable_deadline_at,
    last_activity_at = excluded.last_activity_at,
    closed_at = excluded.closed_at,
    deleted_at = excluded.deleted_at
"#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserRecord {
    pub user_id: String,
    pub principal_kind: String,
    pub principal_subject: String,
    pub created_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRecord {
    pub workspace_id: String,
    pub owner_user_id: String,
    pub name: String,
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
pub enum WorkspaceStoreError {
    Io(String),
    Database(String),
}

impl WorkspaceStoreError {
    pub fn message(&self) -> &str {
        match self {
            Self::Io(message) | Self::Database(message) => message,
        }
    }
}

impl fmt::Display for WorkspaceStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for WorkspaceStoreError {}

#[async_trait]
pub trait WorkspaceStorePort: Send + Sync {
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

#[derive(Debug, Clone)]
pub struct SqliteWorkspaceRepository {
    db_path: Arc<PathBuf>,
}

impl SqliteWorkspaceRepository {
    pub fn new(db_path: impl Into<PathBuf>) -> Result<Self, WorkspaceStoreError> {
        let db_path = db_path.into();
        ensure_parent_dir(&db_path)?;

        let repository = Self {
            db_path: Arc::new(db_path),
        };
        repository.initialize()?;
        Ok(repository)
    }

    fn initialize(&self) -> Result<(), WorkspaceStoreError> {
        let connection = self.open_connection()?;
        connection
            .execute_batch(WORKSPACE_STORE_SCHEMA_SQL)
            .map_err(database_error)?;
        Ok(())
    }

    fn open_connection(&self) -> Result<Connection, WorkspaceStoreError> {
        let connection = Connection::open(self.db_path.as_ref()).map_err(database_error)?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(database_error)?;
        connection
            .pragma_update(None, "foreign_keys", true)
            .map_err(database_error)?;
        Ok(connection)
    }

    fn materialize_user_sync(
        &self,
        principal: &AuthenticatedPrincipal,
    ) -> Result<UserRecord, WorkspaceStoreError> {
        let mut connection = self.open_connection()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        let now = Utc::now();
        let principal_subject = durable_principal_subject(principal);

        let existing = load_user_by_principal(&tx, principal.kind.as_str(), &principal_subject)?;
        let user = if let Some(user) = existing {
            tx.execute(
                "UPDATE users SET last_seen_at = ?1 WHERE user_id = ?2",
                params![timestamp(&now), user.user_id],
            )
            .map_err(database_error)?;
            UserRecord {
                last_seen_at: now,
                ..user
            }
        } else {
            let user = UserRecord {
                user_id: format!("u_{}", uuid::Uuid::new_v4().simple()),
                principal_kind: principal.kind.as_str().to_string(),
                principal_subject,
                created_at: now,
                last_seen_at: now,
            };
            tx.execute(
                "INSERT INTO users (user_id, principal_kind, principal_subject, created_at, last_seen_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    user.user_id,
                    user.principal_kind,
                    user.principal_subject,
                    timestamp(&user.created_at),
                    timestamp(&user.last_seen_at)
                ],
            )
            .map_err(database_error)?;
            user
        };

        tx.commit().map_err(database_error)?;
        Ok(user)
    }

    fn bootstrap_workspace_sync(
        &self,
        owner_user_id: &str,
    ) -> Result<WorkspaceRecord, WorkspaceStoreError> {
        let mut connection = self.open_connection()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        let workspace = bootstrap_workspace_in_transaction(&tx, owner_user_id)?;
        tx.commit().map_err(database_error)?;
        Ok(workspace)
    }

    fn save_session_metadata_sync(
        &self,
        record: &SessionMetadataRecord,
    ) -> Result<(), WorkspaceStoreError> {
        let mut connection = self.open_connection()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        upsert_session_metadata(&tx, record)?;
        tx.commit().map_err(database_error)?;
        Ok(())
    }

    fn persist_session_snapshot_sync(
        &self,
        owner_user_id: &str,
        snapshot: &SessionSnapshot,
        touch_activity: bool,
        status_override: Option<&str>,
    ) -> Result<(), WorkspaceStoreError> {
        let mut connection = self.open_connection()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        let existing = load_session_metadata_record(&tx, owner_user_id, &snapshot.id)?;
        let record = build_session_metadata_record(
            &tx,
            owner_user_id,
            snapshot,
            touch_activity,
            status_override,
            existing.as_ref(),
        )?;
        upsert_session_metadata(&tx, &record)?;
        tx.commit().map_err(database_error)?;
        Ok(())
    }

    fn load_session_metadata_sync(
        &self,
        owner_user_id: &str,
        session_id: &str,
    ) -> Result<Option<SessionMetadataRecord>, WorkspaceStoreError> {
        let connection = self.open_connection()?;
        load_session_metadata_record(&connection, owner_user_id, session_id)
    }
}

#[async_trait]
impl WorkspaceStorePort for SqliteWorkspaceRepository {
    async fn materialize_user(
        &self,
        principal: &AuthenticatedPrincipal,
    ) -> Result<UserRecord, WorkspaceStoreError> {
        let repository = self.clone();
        let principal = principal.clone();
        tokio::task::spawn_blocking(move || repository.materialize_user_sync(&principal))
            .await
            .map_err(join_error)?
    }

    async fn bootstrap_workspace(
        &self,
        owner_user_id: &str,
    ) -> Result<WorkspaceRecord, WorkspaceStoreError> {
        let repository = self.clone();
        let owner_user_id = owner_user_id.to_string();
        tokio::task::spawn_blocking(move || repository.bootstrap_workspace_sync(&owner_user_id))
            .await
            .map_err(join_error)?
    }

    async fn save_session_metadata(
        &self,
        record: &SessionMetadataRecord,
    ) -> Result<(), WorkspaceStoreError> {
        let repository = self.clone();
        let record = record.clone();
        tokio::task::spawn_blocking(move || repository.save_session_metadata_sync(&record))
            .await
            .map_err(join_error)?
    }

    async fn persist_session_snapshot(
        &self,
        owner_user_id: &str,
        snapshot: &SessionSnapshot,
        touch_activity: bool,
        status_override: Option<&str>,
    ) -> Result<(), WorkspaceStoreError> {
        let repository = self.clone();
        let owner_user_id = owner_user_id.to_string();
        let snapshot = snapshot.clone();
        let status_override = status_override.map(str::to_string);
        tokio::task::spawn_blocking(move || {
            repository.persist_session_snapshot_sync(
                &owner_user_id,
                &snapshot,
                touch_activity,
                status_override.as_deref(),
            )
        })
        .await
        .map_err(join_error)?
    }

    async fn load_session_metadata(
        &self,
        owner_user_id: &str,
        session_id: &str,
    ) -> Result<Option<SessionMetadataRecord>, WorkspaceStoreError> {
        let repository = self.clone();
        let owner_user_id = owner_user_id.to_string();
        let session_id = session_id.to_string();
        tokio::task::spawn_blocking(move || {
            repository.load_session_metadata_sync(&owner_user_id, &session_id)
        })
        .await
        .map_err(join_error)?
    }
}

fn ensure_parent_dir(path: &Path) -> Result<(), WorkspaceStoreError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| WorkspaceStoreError::Io(format!("create state directory: {error}")))?;
    }
    Ok(())
}

fn load_user_by_principal(
    connection: &Connection,
    principal_kind: &str,
    principal_subject: &str,
) -> Result<Option<UserRecord>, WorkspaceStoreError> {
    connection
        .query_row(
            "SELECT user_id, principal_kind, principal_subject, created_at, last_seen_at
             FROM users
             WHERE principal_kind = ?1 AND principal_subject = ?2",
            params![principal_kind, principal_subject],
            |row| {
                Ok(UserRecord {
                    user_id: row.get(0)?,
                    principal_kind: row.get(1)?,
                    principal_subject: row.get(2)?,
                    created_at: parse_timestamp_for_row(row.get::<_, String>(3)?, 3)?,
                    last_seen_at: parse_timestamp_for_row(row.get::<_, String>(4)?, 4)?,
                })
            },
        )
        .optional()
        .map_err(database_error)
}

fn load_bootstrap_workspace(
    connection: &Connection,
    owner_user_id: &str,
) -> Result<Option<WorkspaceRecord>, WorkspaceStoreError> {
    connection
        .query_row(
            LOAD_BOOTSTRAP_WORKSPACE_SQL,
            params![owner_user_id, BOOTSTRAP_WORKSPACE_KIND],
            load_workspace_row,
        )
        .optional()
        .map_err(database_error)
}

fn load_workspace_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkspaceRecord> {
    let identity = load_workspace_identity_fields(row)?;
    let timestamps = load_workspace_timestamp_fields(row)?;

    Ok(WorkspaceRecord {
        workspace_id: identity.workspace_id,
        owner_user_id: identity.owner_user_id,
        name: identity.name,
        status: identity.status,
        created_at: timestamps.created_at,
        updated_at: timestamps.updated_at,
        deleted_at: timestamps.deleted_at,
    })
}

struct WorkspaceIdentityFields {
    workspace_id: String,
    owner_user_id: String,
    name: String,
    status: String,
}

fn load_workspace_identity_fields(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<WorkspaceIdentityFields> {
    Ok(WorkspaceIdentityFields {
        workspace_id: row.get(0)?,
        owner_user_id: row.get(1)?,
        name: row.get(2)?,
        status: row.get(3)?,
    })
}

struct WorkspaceTimestampFields {
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    deleted_at: Option<DateTime<Utc>>,
}

fn load_workspace_timestamp_fields(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<WorkspaceTimestampFields> {
    Ok(WorkspaceTimestampFields {
        created_at: parse_timestamp_for_row(row.get::<_, String>(4)?, 4)?,
        updated_at: parse_timestamp_for_row(row.get::<_, String>(5)?, 5)?,
        deleted_at: parse_optional_timestamp_for_row(row.get(6)?, 6)?,
    })
}

fn load_session_metadata_record(
    connection: &Connection,
    owner_user_id: &str,
    session_id: &str,
) -> Result<Option<SessionMetadataRecord>, WorkspaceStoreError> {
    connection
        .query_row(
            LOAD_SESSION_METADATA_SQL,
            params![owner_user_id, session_id],
            load_session_row,
        )
        .optional()
        .map_err(database_error)
}

fn load_session_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionMetadataRecord> {
    let checkout = load_session_checkout_fields(row)?;
    let timing = load_session_timing_fields(row)?;

    Ok(SessionMetadataRecord {
        session_id: row.get(0)?,
        workspace_id: row.get(1)?,
        owner_user_id: row.get(2)?,
        title: row.get(3)?,
        status: row.get(4)?,
        checkout_relpath: checkout.relpath,
        checkout_ref: checkout.reference,
        checkout_commit_sha: checkout.commit_sha,
        failure_reason: row.get(8)?,
        detach_deadline_at: timing.detach_deadline_at,
        restartable_deadline_at: timing.restartable_deadline_at,
        created_at: timing.created_at,
        last_activity_at: timing.last_activity_at,
        closed_at: timing.closed_at,
        deleted_at: timing.deleted_at,
    })
}

struct SessionCheckoutFields {
    relpath: Option<String>,
    reference: Option<String>,
    commit_sha: Option<String>,
}

fn load_session_checkout_fields(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<SessionCheckoutFields> {
    Ok(SessionCheckoutFields {
        relpath: row.get(5)?,
        reference: row.get(6)?,
        commit_sha: row.get(7)?,
    })
}

struct SessionTimingFields {
    detach_deadline_at: Option<DateTime<Utc>>,
    restartable_deadline_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    last_activity_at: DateTime<Utc>,
    closed_at: Option<DateTime<Utc>>,
    deleted_at: Option<DateTime<Utc>>,
}

fn load_session_timing_fields(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionTimingFields> {
    let deadlines = load_session_deadline_fields(row)?;
    let lifecycle = load_session_lifecycle_timestamps(row)?;

    Ok(SessionTimingFields {
        detach_deadline_at: deadlines.detach_deadline_at,
        restartable_deadline_at: deadlines.restartable_deadline_at,
        created_at: lifecycle.created_at,
        last_activity_at: lifecycle.last_activity_at,
        closed_at: lifecycle.closed_at,
        deleted_at: lifecycle.deleted_at,
    })
}

struct SessionDeadlineFields {
    detach_deadline_at: Option<DateTime<Utc>>,
    restartable_deadline_at: Option<DateTime<Utc>>,
}

fn load_session_deadline_fields(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<SessionDeadlineFields> {
    Ok(SessionDeadlineFields {
        detach_deadline_at: parse_optional_timestamp_for_row(row.get(9)?, 9)?,
        restartable_deadline_at: parse_optional_timestamp_for_row(row.get(10)?, 10)?,
    })
}

struct SessionLifecycleTimestamps {
    created_at: DateTime<Utc>,
    last_activity_at: DateTime<Utc>,
    closed_at: Option<DateTime<Utc>>,
    deleted_at: Option<DateTime<Utc>>,
}

fn load_session_lifecycle_timestamps(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<SessionLifecycleTimestamps> {
    Ok(SessionLifecycleTimestamps {
        created_at: parse_timestamp_for_row(row.get::<_, String>(11)?, 11)?,
        last_activity_at: parse_timestamp_for_row(row.get::<_, String>(12)?, 12)?,
        closed_at: parse_optional_timestamp_for_row(row.get(13)?, 13)?,
        deleted_at: parse_optional_timestamp_for_row(row.get(14)?, 14)?,
    })
}

fn timestamp(value: &DateTime<Utc>) -> String {
    value.to_rfc3339()
}

fn parse_timestamp(value: String) -> Result<DateTime<Utc>, WorkspaceStoreError> {
    DateTime::parse_from_rfc3339(&value)
        .map(|timestamp| timestamp.with_timezone(&Utc))
        .map_err(|error| {
            WorkspaceStoreError::Database(format!("invalid timestamp '{value}': {error}"))
        })
}

fn parse_optional_timestamp(
    value: Option<String>,
) -> Result<Option<DateTime<Utc>>, WorkspaceStoreError> {
    value.map(parse_timestamp).transpose()
}

fn parse_timestamp_for_row(value: String, index: usize) -> rusqlite::Result<DateTime<Utc>> {
    parse_timestamp(value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })
}

fn parse_optional_timestamp_for_row(
    value: Option<String>,
    index: usize,
) -> rusqlite::Result<Option<DateTime<Utc>>> {
    parse_optional_timestamp(value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })
}

fn database_error(error: impl fmt::Display) -> WorkspaceStoreError {
    WorkspaceStoreError::Database(error.to_string())
}

fn join_error(error: tokio::task::JoinError) -> WorkspaceStoreError {
    WorkspaceStoreError::Database(format!("blocking workspace task failed: {error}"))
}

impl AuthenticatedPrincipalKind {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Bearer => "bearer",
            Self::BrowserSession => "browser_session",
        }
    }
}

fn durable_principal_subject(principal: &AuthenticatedPrincipal) -> String {
    let namespace = match principal.kind {
        AuthenticatedPrincipalKind::Bearer => DURABLE_BEARER_PRINCIPAL_NAMESPACE,
        AuthenticatedPrincipalKind::BrowserSession => DURABLE_BROWSER_SESSION_PRINCIPAL_NAMESPACE,
    };

    uuid::Uuid::new_v5(&namespace, principal.subject.as_bytes())
        .simple()
        .to_string()
}

fn bootstrap_workspace_in_transaction(
    connection: &Connection,
    owner_user_id: &str,
) -> Result<WorkspaceRecord, WorkspaceStoreError> {
    if let Some(workspace) = load_bootstrap_workspace(connection, owner_user_id)? {
        return Ok(workspace);
    }

    let now = Utc::now();
    let workspace = WorkspaceRecord {
        workspace_id: format!("w_{}", uuid::Uuid::new_v4().simple()),
        owner_user_id: owner_user_id.to_string(),
        name: BOOTSTRAP_WORKSPACE_NAME.to_string(),
        status: ACTIVE_WORKSPACE_STATUS.to_string(),
        created_at: now,
        updated_at: now,
        deleted_at: None,
    };

    connection
        .execute(
            INSERT_BOOTSTRAP_WORKSPACE_SQL,
            params![
                workspace.workspace_id,
                workspace.owner_user_id,
                workspace.name,
                workspace.status,
                BOOTSTRAP_WORKSPACE_KIND,
                timestamp(&workspace.created_at),
                timestamp(&workspace.updated_at)
            ],
        )
        .map_err(database_error)?;

    Ok(workspace)
}

fn upsert_session_metadata(
    connection: &Connection,
    record: &SessionMetadataRecord,
) -> Result<(), WorkspaceStoreError> {
    connection
        .execute(
            UPSERT_SESSION_METADATA_SQL,
            params![
                record.session_id,
                record.workspace_id,
                record.owner_user_id,
                record.title,
                record.status,
                record.checkout_relpath,
                record.checkout_ref,
                record.checkout_commit_sha,
                record.failure_reason,
                record.detach_deadline_at.as_ref().map(timestamp),
                record.restartable_deadline_at.as_ref().map(timestamp),
                timestamp(&record.created_at),
                timestamp(&record.last_activity_at),
                record.closed_at.as_ref().map(timestamp),
                record.deleted_at.as_ref().map(timestamp)
            ],
        )
        .map_err(database_error)?;
    Ok(())
}

fn snapshot_status_name(status: &SessionStatus) -> &'static str {
    match status {
        SessionStatus::Active => "active",
        SessionStatus::Closed => "closed",
    }
}

struct SessionLifecycleState {
    status: String,
    created_at: DateTime<Utc>,
    last_activity_at: DateTime<Utc>,
    closed_at: Option<DateTime<Utc>>,
    deleted_at: Option<DateTime<Utc>>,
}

fn build_session_metadata_record(
    connection: &Connection,
    owner_user_id: &str,
    snapshot: &SessionSnapshot,
    touch_activity: bool,
    status_override: Option<&str>,
    existing: Option<&SessionMetadataRecord>,
) -> Result<SessionMetadataRecord, WorkspaceStoreError> {
    let lifecycle = resolve_session_lifecycle(snapshot, touch_activity, status_override, existing);

    Ok(SessionMetadataRecord {
        session_id: snapshot.id.clone(),
        workspace_id: resolve_workspace_id(connection, owner_user_id, existing)?,
        owner_user_id: owner_user_id.to_string(),
        title: snapshot.title.clone(),
        status: lifecycle.status,
        checkout_relpath: existing.and_then(|record| record.checkout_relpath.clone()),
        checkout_ref: existing.and_then(|record| record.checkout_ref.clone()),
        checkout_commit_sha: existing.and_then(|record| record.checkout_commit_sha.clone()),
        failure_reason: existing.and_then(|record| record.failure_reason.clone()),
        detach_deadline_at: existing.and_then(|record| record.detach_deadline_at),
        restartable_deadline_at: existing.and_then(|record| record.restartable_deadline_at),
        created_at: lifecycle.created_at,
        last_activity_at: lifecycle.last_activity_at,
        closed_at: lifecycle.closed_at,
        deleted_at: lifecycle.deleted_at,
    })
}

fn resolve_workspace_id(
    connection: &Connection,
    owner_user_id: &str,
    existing: Option<&SessionMetadataRecord>,
) -> Result<String, WorkspaceStoreError> {
    match existing {
        Some(record) => Ok(record.workspace_id.clone()),
        None => Ok(bootstrap_workspace_in_transaction(connection, owner_user_id)?.workspace_id),
    }
}

fn resolve_session_lifecycle(
    snapshot: &SessionSnapshot,
    touch_activity: bool,
    status_override: Option<&str>,
    existing: Option<&SessionMetadataRecord>,
) -> SessionLifecycleState {
    let now = Utc::now();
    let created_at = existing.map(|record| record.created_at).unwrap_or(now);
    let status = status_override
        .unwrap_or(snapshot_status_name(&snapshot.status))
        .to_string();

    SessionLifecycleState {
        last_activity_at: if touch_activity {
            now
        } else {
            existing
                .map(|record| record.last_activity_at)
                .unwrap_or(created_at)
        },
        closed_at: resolve_closed_at(existing, &status, now),
        deleted_at: resolve_deleted_at(existing, &status, now),
        status,
        created_at,
    }
}

fn resolve_closed_at(
    existing: Option<&SessionMetadataRecord>,
    status: &str,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    match status {
        "closed" | "deleted" => existing.and_then(|record| record.closed_at).or(Some(now)),
        _ => None,
    }
}

fn resolve_deleted_at(
    existing: Option<&SessionMetadataRecord>,
    status: &str,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    match status {
        "deleted" => Some(now),
        _ => existing.and_then(|record| record.deleted_at),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::sleep;

    fn test_repository() -> SqliteWorkspaceRepository {
        let root = std::env::temp_dir().join(format!(
            "acp-workspace-store-test-{}",
            uuid::Uuid::new_v4().simple()
        ));
        SqliteWorkspaceRepository::new(root.join("db.sqlite"))
            .expect("test workspace repository should initialize")
    }

    fn bearer_principal(subject: &str) -> AuthenticatedPrincipal {
        AuthenticatedPrincipal {
            id: subject.to_string(),
            kind: AuthenticatedPrincipalKind::Bearer,
            subject: subject.to_string(),
        }
    }

    fn browser_principal(subject: &str) -> AuthenticatedPrincipal {
        AuthenticatedPrincipal {
            id: subject.to_string(),
            kind: AuthenticatedPrincipalKind::BrowserSession,
            subject: subject.to_string(),
        }
    }

    fn snapshot(id: &str, title: &str, status: SessionStatus) -> SessionSnapshot {
        SessionSnapshot {
            id: id.to_string(),
            title: title.to_string(),
            status,
            latest_sequence: 0,
            messages: Vec::new(),
            pending_permissions: Vec::new(),
        }
    }

    #[tokio::test]
    async fn principal_materialization_is_stable_and_idempotent() {
        let repository = test_repository();
        let principal = bearer_principal("developer");

        let first = repository
            .materialize_user(&principal)
            .await
            .expect("first materialization should succeed");
        let second = repository
            .materialize_user(&principal)
            .await
            .expect("second materialization should succeed");

        assert_eq!(second.user_id, first.user_id);
        assert_eq!(second.principal_kind, "bearer");
        assert_eq!(second.principal_subject, first.principal_subject);
        assert_ne!(second.principal_subject, "developer");
        assert!(second.last_seen_at >= first.last_seen_at);
    }

    #[tokio::test]
    async fn bootstrap_workspace_creation_is_idempotent() {
        let repository = test_repository();
        let user = repository
            .materialize_user(&bearer_principal("developer"))
            .await
            .expect("principal materialization should succeed");

        let first = repository
            .bootstrap_workspace(&user.user_id)
            .await
            .expect("first bootstrap should succeed");
        let second = repository
            .bootstrap_workspace(&user.user_id)
            .await
            .expect("second bootstrap should succeed");

        assert_eq!(second.workspace_id, first.workspace_id);
        assert_eq!(second.owner_user_id, user.user_id);
        assert_eq!(second.name, BOOTSTRAP_WORKSPACE_NAME);
    }

    #[tokio::test]
    async fn browser_principal_materialization_hashes_the_cookie_subject() {
        let repository = test_repository();
        let principal = browser_principal("11111111-1111-4111-8111-111111111111");

        let first = repository
            .materialize_user(&principal)
            .await
            .expect("first browser materialization should succeed");
        let second = repository
            .materialize_user(&principal)
            .await
            .expect("second browser materialization should succeed");

        assert_eq!(first.user_id, second.user_id);
        assert_eq!(first.principal_kind, "browser_session");
        assert_eq!(first.principal_subject, second.principal_subject);
        assert_ne!(first.principal_subject, principal.subject);
    }

    #[tokio::test]
    async fn session_metadata_can_be_saved_and_loaded_durably() {
        let repository = test_repository();
        let user = repository
            .materialize_user(&bearer_principal("developer"))
            .await
            .expect("principal materialization should succeed");
        let workspace = repository
            .bootstrap_workspace(&user.user_id)
            .await
            .expect("workspace bootstrap should succeed");
        let created_at = Utc::now();
        let record = SessionMetadataRecord {
            session_id: "s_test".to_string(),
            workspace_id: workspace.workspace_id.clone(),
            owner_user_id: user.user_id.clone(),
            title: "Saved session".to_string(),
            status: "closed".to_string(),
            checkout_relpath: None,
            checkout_ref: None,
            checkout_commit_sha: None,
            failure_reason: None,
            detach_deadline_at: None,
            restartable_deadline_at: None,
            created_at,
            last_activity_at: created_at,
            closed_at: Some(created_at),
            deleted_at: None,
        };

        repository
            .save_session_metadata(&record)
            .await
            .expect("saving session metadata should succeed");

        let loaded = repository
            .load_session_metadata(&user.user_id, &record.session_id)
            .await
            .expect("loading session metadata should succeed")
            .expect("saved session metadata should exist");

        assert_eq!(loaded, record);
    }

    #[tokio::test]
    async fn persist_session_snapshot_reuses_workspace_and_preserves_activity_without_touch() {
        let repository = test_repository();
        let user = repository
            .materialize_user(&bearer_principal("developer"))
            .await
            .expect("principal materialization should succeed");
        let snapshot = snapshot("s_persisted", "Initial title", SessionStatus::Active);

        repository
            .persist_session_snapshot(&user.user_id, &snapshot, false, None)
            .await
            .expect("initial persistence should succeed");
        let first = repository
            .load_session_metadata(&user.user_id, &snapshot.id)
            .await
            .expect("initial metadata should load")
            .expect("initial metadata should exist");

        repository
            .persist_session_snapshot(
                &user.user_id,
                &SessionSnapshot {
                    title: "Renamed title".to_string(),
                    ..snapshot.clone()
                },
                false,
                None,
            )
            .await
            .expect("second persistence should succeed");
        let second = repository
            .load_session_metadata(&user.user_id, &snapshot.id)
            .await
            .expect("updated metadata should load")
            .expect("updated metadata should exist");

        assert_eq!(second.workspace_id, first.workspace_id);
        assert_eq!(second.created_at, first.created_at);
        assert_eq!(second.last_activity_at, first.last_activity_at);
        assert_eq!(second.title, "Renamed title");
    }

    #[tokio::test]
    async fn persist_session_snapshot_tracks_activity_close_and_delete_transitions() {
        let repository = test_repository();
        let user = repository
            .materialize_user(&bearer_principal("developer"))
            .await
            .expect("principal materialization should succeed");
        let snapshot = snapshot("s_transition", "Transition", SessionStatus::Active);

        repository
            .persist_session_snapshot(&user.user_id, &snapshot, false, None)
            .await
            .expect("initial persistence should succeed");
        let initial = repository
            .load_session_metadata(&user.user_id, &snapshot.id)
            .await
            .expect("initial metadata should load")
            .expect("initial metadata should exist");

        sleep(Duration::from_millis(5)).await;
        repository
            .persist_session_snapshot(&user.user_id, &snapshot, true, None)
            .await
            .expect("activity touch should succeed");
        let active = repository
            .load_session_metadata(&user.user_id, &snapshot.id)
            .await
            .expect("active metadata should load")
            .expect("active metadata should exist");

        assert!(active.last_activity_at >= initial.last_activity_at);
        assert!(active.closed_at.is_none());
        assert!(active.deleted_at.is_none());

        sleep(Duration::from_millis(5)).await;
        repository
            .persist_session_snapshot(
                &user.user_id,
                &SessionSnapshot {
                    status: SessionStatus::Closed,
                    ..snapshot.clone()
                },
                false,
                None,
            )
            .await
            .expect("close transition should succeed");
        let closed = repository
            .load_session_metadata(&user.user_id, &snapshot.id)
            .await
            .expect("closed metadata should load")
            .expect("closed metadata should exist");

        assert_eq!(closed.status, "closed");
        assert!(closed.closed_at.is_some());
        assert!(closed.deleted_at.is_none());

        sleep(Duration::from_millis(5)).await;
        repository
            .persist_session_snapshot(&user.user_id, &snapshot, false, Some("deleted"))
            .await
            .expect("delete transition should succeed");
        let deleted = repository
            .load_session_metadata(&user.user_id, &snapshot.id)
            .await
            .expect("deleted metadata should load")
            .expect("deleted metadata should exist");

        assert_eq!(deleted.status, "deleted");
        assert!(deleted.closed_at.is_some());
        assert!(deleted.deleted_at.is_some());
    }

    #[tokio::test]
    async fn persist_session_snapshot_surfaces_build_failures_from_broken_schema() {
        let repository = test_repository();
        let user = repository
            .materialize_user(&bearer_principal("developer"))
            .await
            .expect("principal materialization should succeed");
        repository
            .open_connection()
            .expect("opening the test database should succeed")
            .execute("DROP TABLE workspaces", [])
            .expect("dropping workspaces should succeed");

        let error = repository
            .persist_session_snapshot(
                &user.user_id,
                &snapshot("s_broken", "Broken", SessionStatus::Active),
                false,
                None,
            )
            .await
            .expect_err("broken schema should fail");

        assert!(
            matches!(error, WorkspaceStoreError::Database(message) if message.contains("no such table"))
        );
    }

    #[tokio::test]
    async fn workspace_store_error_helpers_preserve_context() {
        let display_error = WorkspaceStoreError::Database("db unavailable".to_string());
        assert_eq!(display_error.to_string(), "db unavailable");

        let mapped_error = database_error("write failed");
        assert_eq!(mapped_error.to_string(), "write failed");

        let parse_error = parse_timestamp("not-a-timestamp".to_string())
            .expect_err("invalid timestamps should fail");
        assert!(parse_error.to_string().contains("invalid timestamp"));

        assert!(matches!(
            parse_timestamp_for_row("not-a-timestamp".to_string(), 11)
                .expect_err("invalid row timestamps should fail"),
            rusqlite::Error::FromSqlConversionFailure(11, rusqlite::types::Type::Text, _)
        ));

        assert!(matches!(
            parse_optional_timestamp_for_row(Some("still-not-a-timestamp".to_string()), 12)
                .expect_err("invalid optional row timestamps should fail"),
            rusqlite::Error::FromSqlConversionFailure(12, rusqlite::types::Type::Text, _)
        ));

        let join_error_value = tokio::spawn(async move { panic!("boom") })
            .await
            .expect_err("panicking tasks should yield join errors");
        let join_mapped = join_error(join_error_value);
        assert!(
            join_mapped
                .to_string()
                .contains("blocking workspace task failed")
        );
    }

    #[test]
    fn ensure_parent_dir_accepts_parentless_paths() {
        assert!(ensure_parent_dir(Path::new("")).is_ok());
    }
}
