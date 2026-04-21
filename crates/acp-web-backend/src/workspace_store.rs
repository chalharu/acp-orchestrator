use std::{
    fmt,
    fmt::Write as _,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use acp_contracts::{LocalAccount, SessionSnapshot, SessionStatus};
use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::auth::{AuthenticatedPrincipal, AuthenticatedPrincipalKind};
use crate::workspace_repository::WorkspaceRepository;

const BOOTSTRAP_WORKSPACE_KIND: &str = "legacy-session-routes";
const BOOTSTRAP_WORKSPACE_NAME: &str = "Default workspace";
const ACTIVE_WORKSPACE_STATUS: &str = "active";
const LOCAL_ACCOUNT_PRINCIPAL_KIND: &str = "local_account";
const LEGACY_BROWSER_SESSIONS_TABLE: &str = "legacy_browser_sessions";
const WORKSPACE_STORE_SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS users (
    user_id TEXT PRIMARY KEY,
    principal_kind TEXT NOT NULL,
    principal_subject TEXT NOT NULL,
    username TEXT,
    password_hash TEXT,
    is_admin INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL,
    deleted_at TEXT,
    UNIQUE(principal_kind, principal_subject)
);

CREATE TABLE IF NOT EXISTS browser_sessions (
    browser_session_id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL,
    deleted_at TEXT,
    FOREIGN KEY (user_id) REFERENCES users(user_id)
);

CREATE INDEX IF NOT EXISTS browser_sessions_user_id_idx
    ON browser_sessions(user_id);

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
        let mut connection = self.open_connection()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        stage_legacy_browser_sessions_table(&tx)?;
        tx.execute_batch(WORKSPACE_STORE_SCHEMA_SQL)
            .map_err(database_error)?;
        ensure_users_column(&tx, "username", "TEXT")?;
        ensure_users_column(&tx, "password_hash", "TEXT")?;
        ensure_users_column(&tx, "is_admin", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_users_column(&tx, "deleted_at", "TEXT")?;
        promote_legacy_bearer_admins(&tx)?;
        migrate_legacy_local_accounts(&tx)?;
        migrate_legacy_browser_sessions(&tx)?;
        drop_legacy_auth_tables(&tx)?;
        tx.execute_batch(
            "DROP INDEX IF EXISTS users_username_idx;
                 CREATE UNIQUE INDEX IF NOT EXISTS users_username_idx
                    ON users(username)
                    WHERE username IS NOT NULL AND deleted_at IS NULL;",
        )
        .map_err(database_error)?;
        tx.commit().map_err(database_error)?;
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
        let user = match principal.kind {
            AuthenticatedPrincipalKind::BrowserSession => {
                materialize_browser_session_user_in_transaction(&tx, &principal.id)?
            }
            AuthenticatedPrincipalKind::Bearer => {
                materialize_bearer_user_in_transaction(&tx, principal)?
            }
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

    fn auth_status_sync(
        &self,
        browser_session_id: Option<&str>,
    ) -> Result<(bool, Option<UserRecord>), WorkspaceStoreError> {
        let connection = self.open_connection()?;
        let bootstrap_required = local_account_count(&connection)? == 0;
        let user = browser_session_id
            .map(|session_id| authenticate_browser_session(&connection, session_id))
            .transpose()?
            .flatten();
        Ok((bootstrap_required, user))
    }

    fn authenticate_browser_session_sync(
        &self,
        browser_session_id: &str,
    ) -> Result<Option<UserRecord>, WorkspaceStoreError> {
        let mut connection = self.open_connection()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        let user = authenticate_browser_session_in_transaction(&tx, browser_session_id)?;
        tx.commit().map_err(database_error)?;
        Ok(user)
    }

    fn bootstrap_local_account_sync(
        &self,
        browser_session_id: &str,
        username: &str,
        password: &str,
    ) -> Result<LocalAccount, WorkspaceStoreError> {
        let mut connection = self.open_connection()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        if local_account_count(&tx)? != 0 {
            return Err(WorkspaceStoreError::Conflict(
                "bootstrap registration is no longer available".to_string(),
            ));
        }
        let account = insert_local_account(&tx, username, password, true)?;
        bind_browser_session_to_user(&tx, browser_session_id, &account.user_id)?;
        tx.commit().map_err(database_error)?;
        Ok(account)
    }

    fn sign_in_local_account_sync(
        &self,
        browser_session_id: &str,
        username: &str,
        password: &str,
    ) -> Result<LocalAccount, WorkspaceStoreError> {
        let mut connection = self.open_connection()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        let user = authenticate_local_account_in_transaction(&tx, username, password)?;
        bind_browser_session_to_user(&tx, browser_session_id, &user.user_id)?;
        tx.commit().map_err(database_error)?;
        local_account_from_user(&user)
    }

    fn list_local_accounts_sync(&self) -> Result<Vec<LocalAccount>, WorkspaceStoreError> {
        let connection = self.open_connection()?;
        list_local_accounts(&connection)
    }

    fn create_local_account_sync(
        &self,
        username: &str,
        password: &str,
        is_admin: bool,
    ) -> Result<LocalAccount, WorkspaceStoreError> {
        let mut connection = self.open_connection()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        let account = insert_local_account(&tx, username, password, is_admin)?;
        tx.commit().map_err(database_error)?;
        Ok(account)
    }

    fn update_local_account_sync(
        &self,
        target_user_id: &str,
        current_user_id: &str,
        password: Option<&str>,
        is_admin: Option<bool>,
    ) -> Result<LocalAccount, WorkspaceStoreError> {
        let mut connection = self.open_connection()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        let account = update_local_account_in_transaction(
            &tx,
            target_user_id,
            current_user_id,
            password,
            is_admin,
        )?;
        tx.commit().map_err(database_error)?;
        Ok(account)
    }

    fn delete_local_account_sync(
        &self,
        target_user_id: &str,
        current_user_id: &str,
    ) -> Result<Vec<String>, WorkspaceStoreError> {
        let mut connection = self.open_connection()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        let browser_session_ids =
            delete_local_account_in_transaction(&tx, target_user_id, current_user_id)?;
        tx.commit().map_err(database_error)?;
        Ok(browser_session_ids)
    }
}

#[async_trait::async_trait]
impl WorkspaceRepository for SqliteWorkspaceRepository {
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

    async fn auth_status(
        &self,
        browser_session_id: Option<&str>,
    ) -> Result<(bool, Option<UserRecord>), WorkspaceStoreError> {
        let repository = self.clone();
        let browser_session_id = browser_session_id.map(str::to_string);
        tokio::task::spawn_blocking(move || {
            repository.auth_status_sync(browser_session_id.as_deref())
        })
        .await
        .map_err(join_error)?
    }

    async fn authenticate_browser_session(
        &self,
        browser_session_id: &str,
    ) -> Result<Option<UserRecord>, WorkspaceStoreError> {
        let repository = self.clone();
        let browser_session_id = browser_session_id.to_string();
        tokio::task::spawn_blocking(move || {
            repository.authenticate_browser_session_sync(&browser_session_id)
        })
        .await
        .map_err(join_error)?
    }

    async fn bootstrap_local_account(
        &self,
        browser_session_id: &str,
        username: &str,
        password: &str,
    ) -> Result<LocalAccount, WorkspaceStoreError> {
        let repository = self.clone();
        let browser_session_id = browser_session_id.to_string();
        let username = username.to_string();
        let password = password.to_string();
        tokio::task::spawn_blocking(move || {
            repository.bootstrap_local_account_sync(&browser_session_id, &username, &password)
        })
        .await
        .map_err(join_error)?
    }

    async fn sign_in_local_account(
        &self,
        browser_session_id: &str,
        username: &str,
        password: &str,
    ) -> Result<LocalAccount, WorkspaceStoreError> {
        let repository = self.clone();
        let browser_session_id = browser_session_id.to_string();
        let username = username.to_string();
        let password = password.to_string();
        tokio::task::spawn_blocking(move || {
            repository.sign_in_local_account_sync(&browser_session_id, &username, &password)
        })
        .await
        .map_err(join_error)?
    }

    async fn list_local_accounts(&self) -> Result<Vec<LocalAccount>, WorkspaceStoreError> {
        let repository = self.clone();
        tokio::task::spawn_blocking(move || repository.list_local_accounts_sync())
            .await
            .map_err(join_error)?
    }

    async fn create_local_account(
        &self,
        username: &str,
        password: &str,
        is_admin: bool,
    ) -> Result<LocalAccount, WorkspaceStoreError> {
        let repository = self.clone();
        let username = username.to_string();
        let password = password.to_string();
        tokio::task::spawn_blocking(move || {
            repository.create_local_account_sync(&username, &password, is_admin)
        })
        .await
        .map_err(join_error)?
    }

    async fn update_local_account(
        &self,
        target_user_id: &str,
        current_user_id: &str,
        password: Option<&str>,
        is_admin: Option<bool>,
    ) -> Result<LocalAccount, WorkspaceStoreError> {
        let repository = self.clone();
        let target_user_id = target_user_id.to_string();
        let current_user_id = current_user_id.to_string();
        let password = password.map(str::to_string);
        tokio::task::spawn_blocking(move || {
            repository.update_local_account_sync(
                &target_user_id,
                &current_user_id,
                password.as_deref(),
                is_admin,
            )
        })
        .await
        .map_err(join_error)?
    }

    async fn delete_local_account(
        &self,
        target_user_id: &str,
        current_user_id: &str,
    ) -> Result<Vec<String>, WorkspaceStoreError> {
        let repository = self.clone();
        let target_user_id = target_user_id.to_string();
        let current_user_id = current_user_id.to_string();
        tokio::task::spawn_blocking(move || {
            repository.delete_local_account_sync(&target_user_id, &current_user_id)
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
            "SELECT user_id, principal_kind, principal_subject, username, password_hash, is_admin, created_at, last_seen_at, deleted_at
             FROM users
             WHERE principal_kind = ?1 AND principal_subject = ?2",
            params![principal_kind, principal_subject],
            load_user_row,
        )
        .optional()
        .map_err(database_error)
}

fn load_user_by_id(
    connection: &Connection,
    user_id: &str,
) -> Result<Option<UserRecord>, WorkspaceStoreError> {
    connection
        .query_row(
            "SELECT user_id, principal_kind, principal_subject, username, password_hash, is_admin, created_at, last_seen_at, deleted_at
             FROM users
             WHERE user_id = ?1",
            params![user_id],
            load_user_row,
        )
        .optional()
        .map_err(database_error)
}

fn load_active_local_account_by_username(
    connection: &Connection,
    username: &str,
) -> Result<Option<UserRecord>, WorkspaceStoreError> {
    connection
        .query_row(
            "SELECT user_id, principal_kind, principal_subject, username, password_hash, is_admin, created_at, last_seen_at, deleted_at
             FROM users
             WHERE username = ?1 AND deleted_at IS NULL",
            params![username],
            load_user_row,
        )
        .optional()
        .map_err(database_error)
}

fn load_user_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<UserRecord> {
    let identity = load_user_identity_fields(row)?;
    let credentials = load_user_credential_fields(row)?;
    let lifecycle = load_user_lifecycle_fields(row)?;

    Ok(UserRecord {
        user_id: identity.user_id,
        principal_kind: identity.principal_kind,
        principal_subject: identity.principal_subject,
        username: credentials.username,
        password_hash: credentials.password_hash,
        is_admin: credentials.is_admin,
        created_at: lifecycle.created_at,
        last_seen_at: lifecycle.last_seen_at,
        deleted_at: lifecycle.deleted_at,
    })
}

struct UserIdentityFields {
    user_id: String,
    principal_kind: String,
    principal_subject: String,
}

fn load_user_identity_fields(row: &rusqlite::Row<'_>) -> rusqlite::Result<UserIdentityFields> {
    Ok(UserIdentityFields {
        user_id: row.get(0)?,
        principal_kind: row.get(1)?,
        principal_subject: row.get(2)?,
    })
}

struct UserCredentialFields {
    username: Option<String>,
    password_hash: Option<String>,
    is_admin: bool,
}

fn load_user_credential_fields(row: &rusqlite::Row<'_>) -> rusqlite::Result<UserCredentialFields> {
    Ok(UserCredentialFields {
        username: row.get(3)?,
        password_hash: row.get(4)?,
        is_admin: row.get::<_, i64>(5)? != 0,
    })
}

struct UserLifecycleFields {
    created_at: DateTime<Utc>,
    last_seen_at: DateTime<Utc>,
    deleted_at: Option<DateTime<Utc>>,
}

fn load_user_lifecycle_fields(row: &rusqlite::Row<'_>) -> rusqlite::Result<UserLifecycleFields> {
    Ok(UserLifecycleFields {
        created_at: parse_timestamp_for_row(row.get::<_, String>(6)?, 6)?,
        last_seen_at: parse_timestamp_for_row(row.get::<_, String>(7)?, 7)?,
        deleted_at: parse_optional_timestamp_for_row(row.get(8)?, 8)?,
    })
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

fn ensure_users_column(
    connection: &Connection,
    column_name: &str,
    column_definition: &str,
) -> Result<(), WorkspaceStoreError> {
    let columns = table_columns(connection, "users")?;
    if columns.iter().any(|column| column == column_name) {
        return Ok(());
    }

    connection
        .execute(
            &format!("ALTER TABLE users ADD COLUMN {column_name} {column_definition}"),
            [],
        )
        .map_err(database_error)?;
    Ok(())
}

fn table_exists(connection: &Connection, table_name: &str) -> Result<bool, WorkspaceStoreError> {
    connection
        .query_row(
            "SELECT 1
             FROM sqlite_master
             WHERE type = 'table' AND name = ?1",
            params![table_name],
            |_| Ok(()),
        )
        .optional()
        .map(|row| row.is_some())
        .map_err(database_error)
}

fn table_columns(
    connection: &Connection,
    table_name: &str,
) -> Result<Vec<String>, WorkspaceStoreError> {
    let mut statement = connection
        .prepare(&format!("PRAGMA table_info({table_name})"))
        .map_err(database_error)?;
    statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(database_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error)
}

fn table_has_column(
    connection: &Connection,
    table_name: &str,
    column_name: &str,
) -> Result<bool, WorkspaceStoreError> {
    Ok(table_columns(connection, table_name)?
        .iter()
        .any(|column| column == column_name))
}

fn stage_legacy_browser_sessions_table(connection: &Connection) -> Result<(), WorkspaceStoreError> {
    if !table_exists(connection, "browser_sessions")?
        || table_has_column(connection, "browser_sessions", "user_id")?
    {
        return Ok(());
    }

    if table_exists(connection, LEGACY_BROWSER_SESSIONS_TABLE)? {
        return Err(WorkspaceStoreError::Database(format!(
            "legacy browser sessions table '{LEGACY_BROWSER_SESSIONS_TABLE}' already exists"
        )));
    }

    connection
        .execute(
            "ALTER TABLE browser_sessions RENAME TO legacy_browser_sessions",
            [],
        )
        .map_err(database_error)?;
    Ok(())
}

#[derive(Debug, Clone)]
struct LegacyLocalAccountRecord {
    username: String,
    password_hash: String,
    is_admin: bool,
    created_at: String,
    updated_at: String,
}

fn load_legacy_local_accounts(
    connection: &Connection,
) -> Result<Option<(Vec<LegacyLocalAccountRecord>, bool)>, WorkspaceStoreError> {
    if !table_exists(connection, "local_accounts")? {
        return Ok(None);
    }

    let has_is_admin = table_has_column(connection, "local_accounts", "is_admin")?;
    let mut statement = connection
        .prepare(if has_is_admin {
            "SELECT user_name, password_hash, is_admin, created_at, updated_at
             FROM local_accounts
             ORDER BY created_at ASC, user_name ASC"
        } else {
            "SELECT user_name, password_hash, 0, created_at, updated_at
             FROM local_accounts
             ORDER BY created_at ASC, user_name ASC"
        })
        .map_err(database_error)?;
    let accounts = statement
        .query_map([], |row| {
            Ok(LegacyLocalAccountRecord {
                username: row.get(0)?,
                password_hash: row.get(1)?,
                is_admin: row.get::<_, i64>(2)? != 0,
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
            })
        })
        .map_err(database_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error)?;
    Ok(Some((accounts, has_is_admin)))
}

fn migrate_legacy_local_accounts(connection: &Connection) -> Result<(), WorkspaceStoreError> {
    let Some((accounts, has_is_admin)) = load_legacy_local_accounts(connection)? else {
        return Ok(());
    };
    let promote_oldest = !has_is_admin || !accounts.iter().any(|account| account.is_admin);

    for (index, account) in accounts.iter().enumerate() {
        let is_admin = account.is_admin || (promote_oldest && index == 0);
        let principal_subject = durable_local_account_subject(&account.username);
        if load_user_by_principal(connection, LOCAL_ACCOUNT_PRINCIPAL_KIND, &principal_subject)?
            .is_some()
        {
            continue;
        }

        connection
            .execute(
                "INSERT INTO users (
                    user_id,
                    principal_kind,
                    principal_subject,
                    username,
                    password_hash,
                    is_admin,
                    created_at,
                    last_seen_at,
                    deleted_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL)",
                params![
                    format!("u_{}", Uuid::new_v4().simple()),
                    LOCAL_ACCOUNT_PRINCIPAL_KIND,
                    principal_subject,
                    &account.username,
                    &account.password_hash,
                    if is_admin { 1 } else { 0 },
                    &account.created_at,
                    &account.updated_at,
                ],
            )
            .map_err(database_error)?;
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct LegacyBrowserSessionRecord {
    browser_session_id: String,
    principal_subject: String,
    created_at: String,
    last_seen_at: String,
}

fn load_legacy_browser_sessions(
    connection: &Connection,
) -> Result<Vec<LegacyBrowserSessionRecord>, WorkspaceStoreError> {
    if !table_exists(connection, LEGACY_BROWSER_SESSIONS_TABLE)? {
        return Ok(Vec::new());
    }

    let mut statement = connection
        .prepare(
            "SELECT session_token, principal_subject, created_at, last_seen_at
             FROM legacy_browser_sessions",
        )
        .map_err(database_error)?;
    statement
        .query_map([], |row| {
            Ok(LegacyBrowserSessionRecord {
                browser_session_id: row.get(0)?,
                principal_subject: row.get(1)?,
                created_at: row.get(2)?,
                last_seen_at: row.get(3)?,
            })
        })
        .map_err(database_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error)
}

fn migrate_legacy_browser_sessions(connection: &Connection) -> Result<(), WorkspaceStoreError> {
    for session in load_legacy_browser_sessions(connection)? {
        let Some(user) =
            load_active_local_account_by_username(connection, &session.principal_subject)?
        else {
            continue;
        };

        connection
            .execute(
                "INSERT INTO browser_sessions (
                    browser_session_id,
                    user_id,
                    created_at,
                    last_seen_at,
                    deleted_at
                 ) VALUES (?1, ?2, ?3, ?4, NULL)
                 ON CONFLICT(browser_session_id) DO UPDATE SET
                    user_id = excluded.user_id,
                    created_at = excluded.created_at,
                    last_seen_at = excluded.last_seen_at,
                    deleted_at = NULL",
                params![
                    session.browser_session_id,
                    user.user_id,
                    session.created_at,
                    session.last_seen_at,
                ],
            )
            .map_err(database_error)?;
    }
    Ok(())
}

fn drop_legacy_auth_tables(connection: &Connection) -> Result<(), WorkspaceStoreError> {
    connection
        .execute("DROP TABLE IF EXISTS local_accounts", [])
        .map_err(database_error)?;
    connection
        .execute("DROP TABLE IF EXISTS legacy_browser_sessions", [])
        .map_err(database_error)?;
    Ok(())
}

fn promote_legacy_bearer_admins(connection: &Connection) -> Result<(), WorkspaceStoreError> {
    connection
        .execute(
            "UPDATE users
             SET is_admin = 1
             WHERE principal_kind = ?1
               AND deleted_at IS NULL",
            params![AuthenticatedPrincipalKind::Bearer.as_str()],
        )
        .map_err(database_error)?;
    Ok(())
}

fn local_account_count(connection: &Connection) -> Result<i64, WorkspaceStoreError> {
    connection
        .query_row(
            "SELECT COUNT(*) FROM users WHERE username IS NOT NULL AND deleted_at IS NULL",
            [],
            |row| row.get(0),
        )
        .map_err(database_error)
}

fn active_admin_count(connection: &Connection) -> Result<i64, WorkspaceStoreError> {
    connection
        .query_row(
            "SELECT COUNT(*) FROM users
             WHERE username IS NOT NULL AND deleted_at IS NULL AND is_admin = 1",
            [],
            |row| row.get(0),
        )
        .map_err(database_error)
}

fn authenticate_browser_session(
    connection: &Connection,
    browser_session_id: &str,
) -> Result<Option<UserRecord>, WorkspaceStoreError> {
    connection
        .query_row(
            "SELECT
                u.user_id,
                u.principal_kind,
                u.principal_subject,
                u.username,
                u.password_hash,
                u.is_admin,
                u.created_at,
                u.last_seen_at,
                u.deleted_at
             FROM browser_sessions bs
             JOIN users u ON u.user_id = bs.user_id
             WHERE bs.browser_session_id = ?1
               AND bs.deleted_at IS NULL
               AND u.deleted_at IS NULL",
            params![browser_session_id],
            load_user_row,
        )
        .optional()
        .map_err(database_error)
}

fn authenticate_browser_session_in_transaction(
    connection: &Connection,
    browser_session_id: &str,
) -> Result<Option<UserRecord>, WorkspaceStoreError> {
    let Some(user) = authenticate_browser_session(connection, browser_session_id)? else {
        return Ok(None);
    };
    let now = Utc::now();
    connection
        .execute(
            "UPDATE browser_sessions SET last_seen_at = ?1 WHERE browser_session_id = ?2",
            params![timestamp(&now), browser_session_id],
        )
        .map_err(database_error)?;
    connection
        .execute(
            "UPDATE users SET last_seen_at = ?1 WHERE user_id = ?2",
            params![timestamp(&now), user.user_id],
        )
        .map_err(database_error)?;
    Ok(Some(UserRecord {
        last_seen_at: now,
        ..user
    }))
}

fn list_local_accounts(connection: &Connection) -> Result<Vec<LocalAccount>, WorkspaceStoreError> {
    let mut statement = connection
        .prepare(
            "SELECT user_id, username, is_admin, created_at
             FROM users
             WHERE username IS NOT NULL AND deleted_at IS NULL
             ORDER BY username ASC",
        )
        .map_err(database_error)?;
    let rows = statement
        .query_map([], |row| {
            Ok(LocalAccount {
                user_id: row.get(0)?,
                username: row.get(1)?,
                is_admin: row.get::<_, i64>(2)? != 0,
                created_at: parse_timestamp_for_row(row.get::<_, String>(3)?, 3)?,
            })
        })
        .map_err(database_error)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(database_error)
}

fn validate_username(username: &str) -> Result<String, WorkspaceStoreError> {
    let username = username.trim();
    if username.is_empty() {
        return Err(WorkspaceStoreError::Validation(
            "username must not be empty".to_string(),
        ));
    }
    if username.chars().count() > 64 {
        return Err(WorkspaceStoreError::Validation(
            "username must not exceed 64 characters".to_string(),
        ));
    }
    if username.chars().any(char::is_whitespace) {
        return Err(WorkspaceStoreError::Validation(
            "username must not contain whitespace".to_string(),
        ));
    }
    Ok(username.to_string())
}

fn validate_password(password: &str) -> Result<(), WorkspaceStoreError> {
    if password.len() < 8 {
        return Err(WorkspaceStoreError::Validation(
            "password must be at least 8 characters".to_string(),
        ));
    }
    Ok(())
}

fn hash_password(password: &str) -> Result<String, WorkspaceStoreError> {
    validate_password(password)?;
    let salt = encode_password_salt(Uuid::new_v4().as_bytes())?;
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|error| WorkspaceStoreError::Database(format!("failed to hash password: {error}")))
}

fn encode_password_salt(bytes: &[u8]) -> Result<SaltString, WorkspaceStoreError> {
    SaltString::encode_b64(bytes)
        .map_err(|error| WorkspaceStoreError::Database(format!("failed to encode salt: {error}")))
}

fn verify_password(password: &str, password_hash: &str) -> Result<bool, WorkspaceStoreError> {
    let parsed = PasswordHash::new(password_hash).map_err(|error| {
        WorkspaceStoreError::Database(format!("invalid password hash: {error}"))
    })?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}

fn authenticate_local_account_in_transaction(
    connection: &Connection,
    username: &str,
    password: &str,
) -> Result<UserRecord, WorkspaceStoreError> {
    let username = username.trim();
    let invalid_credentials =
        || WorkspaceStoreError::Unauthorized("invalid username or password".to_string());
    let Some(user) = load_active_local_account_by_username(connection, username)? else {
        return Err(invalid_credentials());
    };
    let Some(password_hash) = user.password_hash.as_deref() else {
        return Err(invalid_credentials());
    };
    if !verify_password(password, password_hash)? {
        return Err(invalid_credentials());
    }
    let now = Utc::now();
    connection
        .execute(
            "UPDATE users SET last_seen_at = ?1 WHERE user_id = ?2",
            params![timestamp(&now), user.user_id],
        )
        .map_err(database_error)?;
    Ok(UserRecord {
        last_seen_at: now,
        ..user
    })
}

fn materialize_browser_session_user_in_transaction(
    connection: &Connection,
    browser_session_id: &str,
) -> Result<UserRecord, WorkspaceStoreError> {
    authenticate_browser_session_in_transaction(connection, browser_session_id)?.ok_or_else(|| {
        WorkspaceStoreError::Unauthorized("browser session is not linked to an account".to_string())
    })
}

fn materialize_bearer_user_in_transaction(
    connection: &Connection,
    principal: &AuthenticatedPrincipal,
) -> Result<UserRecord, WorkspaceStoreError> {
    let now = Utc::now();
    let principal_subject = durable_principal_subject(principal);
    let existing = load_user_by_principal(connection, principal.kind.as_str(), &principal_subject)?;

    match existing {
        Some(user) => touch_existing_bearer_user(connection, user, now),
        None => insert_bearer_user(connection, principal, principal_subject, now),
    }
}

fn touch_existing_bearer_user(
    connection: &Connection,
    user: UserRecord,
    now: DateTime<Utc>,
) -> Result<UserRecord, WorkspaceStoreError> {
    if user.deleted_at.is_some() {
        return Err(WorkspaceStoreError::Unauthorized(
            "account is no longer available".to_string(),
        ));
    }

    connection
        .execute(
            "UPDATE users SET last_seen_at = ?1 WHERE user_id = ?2",
            params![timestamp(&now), user.user_id],
        )
        .map_err(database_error)?;

    Ok(UserRecord {
        last_seen_at: now,
        ..user
    })
}

fn insert_bearer_user(
    connection: &Connection,
    principal: &AuthenticatedPrincipal,
    principal_subject: String,
    now: DateTime<Utc>,
) -> Result<UserRecord, WorkspaceStoreError> {
    let user = UserRecord {
        user_id: format!("u_{}", uuid::Uuid::new_v4().simple()),
        principal_kind: principal.kind.as_str().to_string(),
        principal_subject,
        username: None,
        password_hash: None,
        is_admin: true,
        created_at: now,
        last_seen_at: now,
        deleted_at: None,
    };
    connection
        .execute(
            "INSERT INTO users (
                user_id,
                principal_kind,
                principal_subject,
                username,
                password_hash,
                is_admin,
                created_at,
                last_seen_at,
                deleted_at
             ) VALUES (?1, ?2, ?3, NULL, NULL, ?4, ?5, ?6, NULL)",
            params![
                user.user_id,
                user.principal_kind,
                user.principal_subject,
                1,
                timestamp(&user.created_at),
                timestamp(&user.last_seen_at)
            ],
        )
        .map_err(database_error)?;
    Ok(user)
}

fn insert_local_account(
    connection: &Connection,
    username: &str,
    password: &str,
    is_admin: bool,
) -> Result<LocalAccount, WorkspaceStoreError> {
    let username = validate_username(username)?;
    let password_hash = hash_password(password)?;
    let now = Utc::now();
    let user_id = format!("u_{}", uuid::Uuid::new_v4().simple());
    let principal_subject = durable_local_account_subject(&username);
    connection
        .execute(
            "INSERT INTO users (
                user_id,
                principal_kind,
                principal_subject,
                username,
                password_hash,
                is_admin,
                created_at,
                last_seen_at,
                deleted_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL)",
            params![
                user_id,
                LOCAL_ACCOUNT_PRINCIPAL_KIND,
                principal_subject,
                username,
                password_hash,
                if is_admin { 1 } else { 0 },
                timestamp(&now),
                timestamp(&now)
            ],
        )
        .map_err(map_account_write_error)?;
    Ok(LocalAccount {
        user_id,
        username,
        is_admin,
        created_at: now,
    })
}

fn bind_browser_session_to_user(
    connection: &Connection,
    browser_session_id: &str,
    user_id: &str,
) -> Result<(), WorkspaceStoreError> {
    let now = Utc::now();
    connection
        .execute(
            "INSERT INTO browser_sessions (
                browser_session_id,
                user_id,
                created_at,
                last_seen_at,
                deleted_at
             ) VALUES (?1, ?2, ?3, ?4, NULL)
             ON CONFLICT(browser_session_id) DO UPDATE SET
                user_id = excluded.user_id,
                last_seen_at = excluded.last_seen_at,
                deleted_at = NULL",
            params![
                browser_session_id,
                user_id,
                timestamp(&now),
                timestamp(&now)
            ],
        )
        .map_err(database_error)?;
    Ok(())
}

fn active_local_account(
    connection: &Connection,
    user_id: &str,
) -> Result<UserRecord, WorkspaceStoreError> {
    let user = load_user_by_id(connection, user_id)?
        .filter(|user| user.deleted_at.is_none() && user.username.is_some())
        .ok_or_else(|| WorkspaceStoreError::NotFound("account not found".to_string()))?;
    Ok(user)
}

fn local_account_from_user(user: &UserRecord) -> Result<LocalAccount, WorkspaceStoreError> {
    let username = user
        .username
        .clone()
        .ok_or_else(|| WorkspaceStoreError::NotFound("account not found".to_string()))?;
    Ok(LocalAccount {
        user_id: user.user_id.clone(),
        username,
        is_admin: user.is_admin,
        created_at: user.created_at,
    })
}

fn update_local_account_in_transaction(
    connection: &Connection,
    target_user_id: &str,
    current_user_id: &str,
    password: Option<&str>,
    is_admin: Option<bool>,
) -> Result<LocalAccount, WorkspaceStoreError> {
    let target = active_local_account(connection, target_user_id)?;
    validate_local_account_admin_change(connection, &target, current_user_id, is_admin)?;
    let password_hash = next_password_hash(password)?;
    let is_admin = is_admin.unwrap_or(target.is_admin);
    persist_local_account_update(connection, target_user_id, password_hash, is_admin)?;
    let updated = active_local_account(connection, target_user_id)?;
    local_account_from_user(&updated)
}

fn delete_local_account_in_transaction(
    connection: &Connection,
    target_user_id: &str,
    current_user_id: &str,
) -> Result<Vec<String>, WorkspaceStoreError> {
    let target = active_local_account(connection, target_user_id)?;
    validate_local_account_deletion(connection, &target, current_user_id)?;
    let browser_session_ids = active_browser_session_ids_for_user(connection, target_user_id)?;
    let now = Utc::now();
    soft_delete_browser_sessions(connection, target_user_id, now)?;
    soft_delete_local_account(connection, target_user_id, now)?;
    Ok(browser_session_ids)
}

fn validate_local_account_admin_change(
    connection: &Connection,
    target: &UserRecord,
    current_user_id: &str,
    is_admin: Option<bool>,
) -> Result<(), WorkspaceStoreError> {
    let Some(make_admin) = is_admin else {
        return Ok(());
    };

    if target.user_id == current_user_id && !make_admin {
        return Err(WorkspaceStoreError::Conflict(
            "signed-in account cannot remove its own admin access".to_string(),
        ));
    }
    if target.is_admin && !make_admin && active_admin_count(connection)? <= 1 {
        return Err(WorkspaceStoreError::Conflict(
            "at least one admin account must remain".to_string(),
        ));
    }
    Ok(())
}

fn next_password_hash(password: Option<&str>) -> Result<Option<String>, WorkspaceStoreError> {
    match password {
        Some(password) if !password.trim().is_empty() => hash_password(password).map(Some),
        Some(_) => Err(WorkspaceStoreError::Validation(
            "password must not be empty".to_string(),
        )),
        None => Ok(None),
    }
}

fn persist_local_account_update(
    connection: &Connection,
    target_user_id: &str,
    next_password_hash: Option<String>,
    next_is_admin: bool,
) -> Result<(), WorkspaceStoreError> {
    connection
        .execute(
            "UPDATE users
             SET password_hash = COALESCE(?1, password_hash),
                 is_admin = ?2
             WHERE user_id = ?3",
            params![
                next_password_hash,
                if next_is_admin { 1 } else { 0 },
                target_user_id
            ],
        )
        .map_err(database_error)?;
    Ok(())
}

fn validate_local_account_deletion(
    connection: &Connection,
    target: &UserRecord,
    current_user_id: &str,
) -> Result<(), WorkspaceStoreError> {
    if target.user_id == current_user_id {
        return Err(WorkspaceStoreError::Conflict(
            "signed-in account cannot be deleted".to_string(),
        ));
    }
    if target.is_admin && active_admin_count(connection)? <= 1 {
        return Err(WorkspaceStoreError::Conflict(
            "at least one admin account must remain".to_string(),
        ));
    }
    Ok(())
}

fn active_browser_session_ids_for_user(
    connection: &Connection,
    user_id: &str,
) -> Result<Vec<String>, WorkspaceStoreError> {
    let mut statement = connection
        .prepare(
            "SELECT browser_session_id
             FROM browser_sessions
             WHERE user_id = ?1 AND deleted_at IS NULL",
        )
        .map_err(database_error)?;
    statement
        .query_map(params![user_id], |row| row.get::<_, String>(0))
        .map_err(database_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error)
}

fn soft_delete_browser_sessions(
    connection: &Connection,
    target_user_id: &str,
    now: DateTime<Utc>,
) -> Result<(), WorkspaceStoreError> {
    connection
        .execute(
            "UPDATE browser_sessions SET deleted_at = ?1 WHERE user_id = ?2 AND deleted_at IS NULL",
            params![timestamp(&now), target_user_id],
        )
        .map_err(database_error)?;
    Ok(())
}

fn soft_delete_local_account(
    connection: &Connection,
    target_user_id: &str,
    now: DateTime<Utc>,
) -> Result<(), WorkspaceStoreError> {
    connection
        .execute(
            "UPDATE users
             SET deleted_at = ?1,
                 username = NULL,
                 password_hash = NULL,
                 principal_subject = ?2
             WHERE user_id = ?3",
            params![
                timestamp(&now),
                deleted_local_account_subject(target_user_id),
                target_user_id
            ],
        )
        .map_err(database_error)?;
    Ok(())
}

fn durable_local_account_subject(username: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(LOCAL_ACCOUNT_PRINCIPAL_KIND.as_bytes());
    digest.update([0]);
    digest.update(username.as_bytes());
    let digest = digest.finalize();
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
}

fn deleted_local_account_subject(user_id: &str) -> String {
    format!("deleted-local-account:{user_id}")
}

fn map_account_write_error(error: rusqlite::Error) -> WorkspaceStoreError {
    match error {
        rusqlite::Error::SqliteFailure(_, Some(message)) => {
            if message.contains("users.username") || message.contains("users.username_idx") {
                WorkspaceStoreError::Conflict("username already exists".to_string())
            } else {
                WorkspaceStoreError::Database(message)
            }
        }
        other => database_error(other),
    }
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
    let mut digest = Sha256::new();
    digest.update(principal.kind.as_str().as_bytes());
    digest.update([0]);
    digest.update(principal.subject.as_bytes());
    let digest = digest.finalize();
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
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

    fn legacy_user_db_connection(label: &str, schema: &str) -> (std::path::PathBuf, Connection) {
        let db_path = std::env::temp_dir()
            .join(format!(
                "acp-web-backend-{label}-{}",
                uuid::Uuid::new_v4().simple()
            ))
            .join("db.sqlite");
        ensure_parent_dir(&db_path).expect("legacy db parent should initialize");
        let connection = Connection::open(&db_path).expect("legacy database should open");
        connection
            .execute_batch(schema)
            .expect("legacy users table should initialize");
        (db_path, connection)
    }

    fn insert_legacy_bearer_user(
        connection: &Connection,
        user_id: &str,
        principal: &AuthenticatedPrincipal,
    ) {
        let now = timestamp(&Utc::now());
        connection
            .execute(
                "INSERT INTO users (
                    user_id,
                    principal_kind,
                    principal_subject,
                    created_at,
                    last_seen_at
                ) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    user_id,
                    AuthenticatedPrincipalKind::Bearer.as_str(),
                    durable_principal_subject(principal),
                    now,
                    now
                ],
            )
            .expect("legacy bearer user should insert");
    }

    fn insert_legacy_bearer_user_with_auth_columns(
        connection: &Connection,
        user_id: &str,
        principal: &AuthenticatedPrincipal,
    ) {
        let now = timestamp(&Utc::now());
        connection
            .execute(
                "INSERT INTO users (
                    user_id,
                    principal_kind,
                    principal_subject,
                    created_at,
                    last_seen_at,
                    username,
                    password_hash,
                    is_admin,
                    deleted_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL, 0, NULL)",
                params![
                    user_id,
                    AuthenticatedPrincipalKind::Bearer.as_str(),
                    durable_principal_subject(principal),
                    now,
                    now
                ],
            )
            .expect("legacy bearer user should insert");
    }

    fn insert_legacy_local_account(
        connection: &Connection,
        username: &str,
        password: &str,
        created_at: &str,
        updated_at: &str,
        is_admin: Option<bool>,
    ) {
        let password_hash = hash_password(password).expect("legacy password hash should encode");
        match is_admin {
            Some(is_admin) => {
                connection
                    .execute(
                        "INSERT INTO local_accounts (
                            user_name,
                            password_hash,
                            is_admin,
                            created_at,
                            updated_at
                        ) VALUES (?1, ?2, ?3, ?4, ?5)",
                        params![
                            username,
                            password_hash,
                            if is_admin { 1 } else { 0 },
                            created_at,
                            updated_at
                        ],
                    )
                    .expect("legacy local account with admin flag should insert");
            }
            None => {
                connection
                    .execute(
                        "INSERT INTO local_accounts (
                            user_name,
                            password_hash,
                            created_at,
                            updated_at
                        ) VALUES (?1, ?2, ?3, ?4)",
                        params![username, password_hash, created_at, updated_at],
                    )
                    .expect("legacy local account should insert");
            }
        }
    }

    fn insert_legacy_browser_session(
        connection: &Connection,
        browser_session_id: &str,
        username: &str,
        created_at: &str,
        last_seen_at: &str,
    ) {
        connection
            .execute(
                "INSERT INTO browser_sessions (
                    session_token,
                    principal_subject,
                    created_at,
                    last_seen_at
                ) VALUES (?1, ?2, ?3, ?4)",
                params![browser_session_id, username, created_at, last_seen_at],
            )
            .expect("legacy browser session should insert");
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
        assert!(second.is_admin);
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
    async fn browser_principal_materialization_requires_an_authenticated_account() {
        let repository = test_repository();
        let principal = browser_principal("11111111-1111-4111-8111-111111111111");

        let error = repository
            .materialize_user(&principal)
            .await
            .expect_err("browser materialization should require an authenticated account");

        assert!(matches!(error, WorkspaceStoreError::Unauthorized(_)));
    }

    #[tokio::test]
    async fn browser_principal_materializes_authenticated_account() {
        let repository = test_repository();
        let account = repository
            .bootstrap_local_account(
                "11111111-1111-4111-8111-111111111111",
                "admin",
                "password123",
            )
            .await
            .expect("bootstrap should succeed");

        let materialized = repository
            .materialize_user(&browser_principal("11111111-1111-4111-8111-111111111111"))
            .await
            .expect("browser principal should materialize the linked account");

        assert_eq!(materialized.user_id, account.user_id);
        assert_eq!(materialized.username.as_deref(), Some("admin"));
        assert_eq!(materialized.principal_kind, LOCAL_ACCOUNT_PRINCIPAL_KIND);
        assert!(materialized.is_admin);
    }

    #[tokio::test]
    async fn bootstrap_account_binds_the_browser_session_and_lists_accounts() {
        let repository = test_repository();
        let account = repository
            .bootstrap_local_account(
                "11111111-1111-4111-8111-111111111111",
                "admin",
                "password123",
            )
            .await
            .expect("bootstrap should succeed");
        let authenticated = repository
            .authenticate_browser_session("11111111-1111-4111-8111-111111111111")
            .await
            .expect("authentication should succeed")
            .expect("browser session should be linked");
        let listed = repository
            .list_local_accounts()
            .await
            .expect("listing accounts should succeed");

        assert_eq!(account.username, "admin");
        assert!(account.is_admin);
        assert_eq!(authenticated.user_id, account.user_id);
        assert_eq!(listed, vec![account]);
    }

    #[tokio::test]
    async fn signing_in_rebinds_a_browser_session_to_an_existing_account() {
        let repository = test_repository();
        let account = repository
            .bootstrap_local_account(
                "11111111-1111-4111-8111-111111111111",
                "admin",
                "password123",
            )
            .await
            .expect("bootstrap should succeed");

        let signed_in = repository
            .sign_in_local_account(
                "22222222-2222-4222-8222-222222222222",
                "admin",
                "password123",
            )
            .await
            .expect("sign-in should succeed");
        let authenticated = repository
            .authenticate_browser_session("22222222-2222-4222-8222-222222222222")
            .await
            .expect("authentication should succeed")
            .expect("browser session should be linked");

        assert_eq!(signed_in, account);
        assert_eq!(authenticated.user_id, account.user_id);
    }

    #[tokio::test]
    async fn signing_in_rejects_invalid_credentials() {
        let repository = test_repository();
        repository
            .bootstrap_local_account(
                "11111111-1111-4111-8111-111111111111",
                "admin",
                "password123",
            )
            .await
            .expect("bootstrap should succeed");

        let error = repository
            .sign_in_local_account(
                "22222222-2222-4222-8222-222222222222",
                "admin",
                "wrong-password",
            )
            .await
            .expect_err("sign-in should reject invalid credentials");

        assert_eq!(
            error,
            WorkspaceStoreError::Unauthorized("invalid username or password".to_string())
        );

        let missing = repository
            .sign_in_local_account(
                "22222222-2222-4222-8222-222222222222",
                "missing",
                "password123",
            )
            .await
            .expect_err("sign-in should reject missing accounts");

        assert_eq!(
            missing,
            WorkspaceStoreError::Unauthorized("invalid username or password".to_string())
        );
    }

    #[tokio::test]
    async fn bootstrap_registration_is_rejected_after_the_first_account() {
        let repository = test_repository();
        repository
            .bootstrap_local_account(
                "11111111-1111-4111-8111-111111111111",
                "admin",
                "password123",
            )
            .await
            .expect("bootstrap should succeed");

        let error = repository
            .bootstrap_local_account(
                "22222222-2222-4222-8222-222222222222",
                "second-admin",
                "password123",
            )
            .await
            .expect_err("bootstrap should close after the first account");

        assert_eq!(
            error,
            WorkspaceStoreError::Conflict(
                "bootstrap registration is no longer available".to_string()
            )
        );
    }

    #[test]
    fn validation_helpers_reject_invalid_usernames_and_passwords() {
        assert_eq!(
            validate_username("   ").expect_err("blank usernames should fail"),
            WorkspaceStoreError::Validation("username must not be empty".to_string())
        );
        assert_eq!(
            validate_username(&"a".repeat(65)).expect_err("long usernames should fail"),
            WorkspaceStoreError::Validation("username must not exceed 64 characters".to_string())
        );
        assert_eq!(
            validate_username("two words").expect_err("whitespace should fail"),
            WorkspaceStoreError::Validation("username must not contain whitespace".to_string())
        );
        assert_eq!(
            validate_password("short").expect_err("short passwords should fail"),
            WorkspaceStoreError::Validation("password must be at least 8 characters".to_string())
        );
        assert_eq!(
            next_password_hash(None).expect("missing passwords should be allowed"),
            None
        );
    }

    #[test]
    fn invalid_password_hashes_and_write_errors_are_reported_clearly() {
        let salt_error =
            encode_password_salt(&[0; 100]).expect_err("oversized salts should fail to encode");
        assert!(matches!(
            salt_error,
            WorkspaceStoreError::Database(message) if message.contains("failed to encode salt")
        ));

        let invalid_hash = verify_password("password123", "invalid-hash")
            .expect_err("invalid password hashes should fail");
        assert!(matches!(
            invalid_hash,
            WorkspaceStoreError::Database(message) if message.contains("invalid password hash")
        ));

        let conflict = map_account_write_error(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(1),
            Some("UNIQUE constraint failed: users.username_idx".to_string()),
        ));
        assert_eq!(
            conflict,
            WorkspaceStoreError::Conflict("username already exists".to_string())
        );

        let database = map_account_write_error(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(1),
            Some("other failure".to_string()),
        ));
        assert_eq!(
            database,
            WorkspaceStoreError::Database("other failure".to_string())
        );

        assert!(matches!(
            map_account_write_error(rusqlite::Error::InvalidQuery),
            WorkspaceStoreError::Database(_)
        ));
    }

    #[tokio::test]
    async fn signing_in_rejects_accounts_without_password_hashes() {
        let repository = test_repository();
        repository
            .bootstrap_local_account(
                "11111111-1111-4111-8111-111111111111",
                "admin",
                "password123",
            )
            .await
            .expect("bootstrap should succeed");
        repository
            .open_connection()
            .expect("opening the test database should succeed")
            .execute(
                "INSERT INTO users (
                    user_id,
                    principal_kind,
                    principal_subject,
                    username,
                    password_hash,
                    is_admin,
                    created_at,
                    last_seen_at,
                    deleted_at
                ) VALUES (?1, ?2, ?3, ?4, NULL, 0, ?5, ?5, NULL)",
                params![
                    "u_member",
                    LOCAL_ACCOUNT_PRINCIPAL_KIND,
                    "local-account:member",
                    "member",
                    timestamp(&Utc::now()),
                ],
            )
            .expect("local account without a password hash should insert");

        let error = repository
            .sign_in_local_account(
                "22222222-2222-4222-8222-222222222222",
                "member",
                "password123",
            )
            .await
            .expect_err("sign-in should reject accounts without a password hash");

        assert_eq!(
            error,
            WorkspaceStoreError::Unauthorized("invalid username or password".to_string())
        );
    }

    #[tokio::test]
    async fn updating_an_account_can_change_password_and_admin_access() {
        let repository = test_repository();
        let admin = repository
            .bootstrap_local_account(
                "11111111-1111-4111-8111-111111111111",
                "admin",
                "password123",
            )
            .await
            .expect("bootstrap should succeed");
        let member = repository
            .create_local_account("member", "password123", false)
            .await
            .expect("secondary account creation should succeed");

        let updated = repository
            .update_local_account(
                &member.user_id,
                &admin.user_id,
                Some("password456"),
                Some(true),
            )
            .await
            .expect("account updates should succeed");
        let signed_in = repository
            .sign_in_local_account(
                "22222222-2222-4222-8222-222222222222",
                "member",
                "password456",
            )
            .await
            .expect("the updated password should authenticate");

        assert!(updated.is_admin);
        assert_eq!(signed_in.user_id, member.user_id);
    }

    #[tokio::test]
    async fn updating_an_account_can_keep_existing_admin_access() {
        let repository = test_repository();
        let admin = repository
            .bootstrap_local_account(
                "11111111-1111-4111-8111-111111111111",
                "admin",
                "password123",
            )
            .await
            .expect("bootstrap should succeed");
        let member = repository
            .create_local_account("member", "password123", false)
            .await
            .expect("secondary account creation should succeed");

        let updated = repository
            .update_local_account(&member.user_id, &admin.user_id, Some("password456"), None)
            .await
            .expect("password-only updates should succeed");

        assert!(!updated.is_admin);
    }

    #[tokio::test]
    async fn deleted_bearer_accounts_cannot_be_materialized_again() {
        let repository = test_repository();
        let principal = bearer_principal("developer");
        let user = repository
            .materialize_user(&principal)
            .await
            .expect("initial materialization should succeed");

        repository
            .open_connection()
            .expect("opening the test database should succeed")
            .execute(
                "UPDATE users SET deleted_at = ?1 WHERE user_id = ?2",
                params![timestamp(&Utc::now()), user.user_id],
            )
            .expect("soft deleting the user should succeed");

        let error = repository
            .materialize_user(&principal)
            .await
            .expect_err("deleted bearer users should not rematerialize");

        assert_eq!(
            error,
            WorkspaceStoreError::Unauthorized("account is no longer available".to_string())
        );
    }

    #[tokio::test]
    async fn initialization_migrates_legacy_users_before_creating_the_username_index() {
        let (db_path, connection) = legacy_user_db_connection(
            "legacy-users",
            "CREATE TABLE users (
                    user_id TEXT PRIMARY KEY,
                    principal_kind TEXT NOT NULL,
                    principal_subject TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    last_seen_at TEXT NOT NULL
                );",
        );
        insert_legacy_bearer_user(&connection, "u_legacy", &bearer_principal("developer"));
        drop(connection);

        let repository =
            SqliteWorkspaceRepository::new(db_path).expect("repository should migrate");
        let user = repository
            .materialize_user(&bearer_principal("developer"))
            .await
            .expect("materialization should succeed after migration");

        assert!(user.is_admin);
    }

    #[tokio::test]
    async fn initialization_promotes_existing_bearer_users_to_admin() {
        let (db_path, connection) = legacy_user_db_connection(
            "bearer-admin-migration",
            "CREATE TABLE users (
                    user_id TEXT PRIMARY KEY,
                    principal_kind TEXT NOT NULL,
                    principal_subject TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    last_seen_at TEXT NOT NULL,
                    username TEXT,
                    password_hash TEXT,
                    is_admin INTEGER NOT NULL DEFAULT 0,
                    deleted_at TEXT
                );",
        );
        insert_legacy_bearer_user_with_auth_columns(
            &connection,
            "u_legacy",
            &bearer_principal("developer"),
        );
        drop(connection);

        let repository =
            SqliteWorkspaceRepository::new(db_path).expect("repository should migrate");
        let user = repository
            .materialize_user(&bearer_principal("developer"))
            .await
            .expect("materialization should succeed after migration");

        assert!(user.is_admin);
    }

    #[tokio::test]
    async fn initialization_migrates_legacy_local_accounts_and_browser_sessions_without_admin_flags()
     {
        let (db_path, connection) = legacy_user_db_connection(
            "legacy-local-accounts-no-admin",
            "CREATE TABLE local_accounts (
                    user_name TEXT PRIMARY KEY,
                    password_hash TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE TABLE browser_sessions (
                    session_token TEXT PRIMARY KEY,
                    principal_subject TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    last_seen_at TEXT NOT NULL
                );",
        );
        insert_legacy_local_account(
            &connection,
            "alice",
            "password123",
            "2024-01-01T00:00:00Z",
            "2024-01-01T01:00:00Z",
            None,
        );
        insert_legacy_local_account(
            &connection,
            "bob",
            "password456",
            "2024-01-02T00:00:00Z",
            "2024-01-02T01:00:00Z",
            None,
        );
        insert_legacy_browser_session(
            &connection,
            "legacy-session-alice",
            "alice",
            "2024-01-03T00:00:00Z",
            "2024-01-03T01:00:00Z",
        );
        insert_legacy_browser_session(
            &connection,
            "legacy-session-missing",
            "missing",
            "2024-01-03T00:00:00Z",
            "2024-01-03T01:00:00Z",
        );
        drop(connection);

        let repository =
            SqliteWorkspaceRepository::new(db_path).expect("repository should migrate");
        let accounts = repository
            .list_local_accounts()
            .await
            .expect("listing migrated accounts should succeed");
        let alice = accounts
            .iter()
            .find(|account| account.username == "alice")
            .expect("alice should be migrated");
        let bob = accounts
            .iter()
            .find(|account| account.username == "bob")
            .expect("bob should be migrated");
        let authenticated = repository
            .authenticate_browser_session("legacy-session-alice")
            .await
            .expect("legacy browser session should authenticate")
            .expect("alice session should be preserved");
        let missing = repository
            .authenticate_browser_session("legacy-session-missing")
            .await
            .expect("orphaned legacy browser session lookup should succeed");
        let signed_in = repository
            .sign_in_local_account("fresh-session", "alice", "password123")
            .await
            .expect("migrated account should preserve the password hash");

        assert!(alice.is_admin);
        assert!(!bob.is_admin);
        assert_eq!(authenticated.user_id, alice.user_id);
        assert_eq!(authenticated.username.as_deref(), Some("alice"));
        assert!(missing.is_none());
        assert_eq!(signed_in.user_id, alice.user_id);
    }

    #[tokio::test]
    async fn initialization_migrates_legacy_local_accounts_and_browser_sessions_with_admin_flags() {
        let (db_path, connection) = legacy_user_db_connection(
            "legacy-local-accounts-with-admin",
            "CREATE TABLE local_accounts (
                    user_name TEXT PRIMARY KEY,
                    password_hash TEXT NOT NULL,
                    is_admin INTEGER NOT NULL DEFAULT 0,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE TABLE browser_sessions (
                    session_token TEXT PRIMARY KEY,
                    principal_subject TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    last_seen_at TEXT NOT NULL
                );",
        );
        insert_legacy_local_account(
            &connection,
            "alice",
            "password123",
            "2024-01-01T00:00:00Z",
            "2024-01-01T01:00:00Z",
            Some(false),
        );
        insert_legacy_local_account(
            &connection,
            "bob",
            "password456",
            "2024-01-02T00:00:00Z",
            "2024-01-02T01:00:00Z",
            Some(true),
        );
        insert_legacy_browser_session(
            &connection,
            "legacy-session-bob",
            "bob",
            "2024-01-03T00:00:00Z",
            "2024-01-03T01:00:00Z",
        );
        drop(connection);

        let repository =
            SqliteWorkspaceRepository::new(db_path).expect("repository should migrate");
        let accounts = repository
            .list_local_accounts()
            .await
            .expect("listing migrated accounts should succeed");
        let alice = accounts
            .iter()
            .find(|account| account.username == "alice")
            .expect("alice should be migrated");
        let bob = accounts
            .iter()
            .find(|account| account.username == "bob")
            .expect("bob should be migrated");
        let authenticated = repository
            .authenticate_browser_session("legacy-session-bob")
            .await
            .expect("legacy browser session should authenticate")
            .expect("bob session should be preserved");
        let signed_in = repository
            .sign_in_local_account("fresh-session-bob", "bob", "password456")
            .await
            .expect("migrated admin account should preserve the password hash");

        assert!(!alice.is_admin);
        assert!(bob.is_admin);
        assert_eq!(authenticated.user_id, bob.user_id);
        assert_eq!(authenticated.username.as_deref(), Some("bob"));
        assert!(authenticated.is_admin);
        assert_eq!(signed_in.user_id, bob.user_id);
    }

    #[tokio::test]
    async fn initialization_does_not_overwrite_current_local_accounts_when_legacy_tables_remain() {
        let (db_path, connection) = legacy_user_db_connection(
            "mixed-current-and-legacy-local-accounts",
            "CREATE TABLE users (
                    user_id TEXT PRIMARY KEY,
                    principal_kind TEXT NOT NULL,
                    principal_subject TEXT NOT NULL,
                    username TEXT,
                    password_hash TEXT,
                    is_admin INTEGER NOT NULL DEFAULT 0,
                    created_at TEXT NOT NULL,
                    last_seen_at TEXT NOT NULL,
                    deleted_at TEXT,
                    UNIQUE(principal_kind, principal_subject)
                );
                CREATE TABLE local_accounts (
                    user_name TEXT PRIMARY KEY,
                    password_hash TEXT NOT NULL,
                    is_admin INTEGER NOT NULL DEFAULT 0,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE TABLE browser_sessions (
                    session_token TEXT PRIMARY KEY,
                    principal_subject TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    last_seen_at TEXT NOT NULL
                );",
        );
        let current_password_hash =
            hash_password("password-new").expect("current password hash should build");
        let now = timestamp(&Utc::now());
        connection
            .execute(
                "INSERT INTO users (
                    user_id,
                    principal_kind,
                    principal_subject,
                    username,
                    password_hash,
                    is_admin,
                    created_at,
                    last_seen_at,
                    deleted_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, ?6, NULL)",
                params![
                    "u_current_alice",
                    LOCAL_ACCOUNT_PRINCIPAL_KIND,
                    durable_local_account_subject("alice"),
                    "alice",
                    current_password_hash,
                    now,
                ],
            )
            .expect("current local account should insert");
        insert_legacy_local_account(
            &connection,
            "alice",
            "password-old",
            "2024-01-01T00:00:00Z",
            "2024-01-01T01:00:00Z",
            Some(true),
        );
        insert_legacy_browser_session(
            &connection,
            "legacy-session-alice",
            "alice",
            "2024-01-03T00:00:00Z",
            "2024-01-03T01:00:00Z",
        );
        drop(connection);

        let repository =
            SqliteWorkspaceRepository::new(db_path).expect("repository should migrate");
        let accounts = repository
            .list_local_accounts()
            .await
            .expect("listing migrated accounts should succeed");
        let alice = accounts
            .iter()
            .find(|account| account.username == "alice")
            .expect("alice should remain available");
        let authenticated = repository
            .authenticate_browser_session("legacy-session-alice")
            .await
            .expect("legacy browser session should authenticate")
            .expect("alice session should be preserved");
        let signed_in = repository
            .sign_in_local_account("fresh-session-alice", "alice", "password-new")
            .await
            .expect("current password should remain authoritative");
        let old_password_error = repository
            .sign_in_local_account("rejected-session-alice", "alice", "password-old")
            .await
            .expect_err("stale legacy password should not replace the current one");

        assert_eq!(accounts.len(), 1);
        assert!(!alice.is_admin);
        assert_eq!(authenticated.user_id, alice.user_id);
        assert_eq!(signed_in.user_id, alice.user_id);
        assert_eq!(
            old_password_error,
            WorkspaceStoreError::Unauthorized("invalid username or password".to_string())
        );
    }

    #[tokio::test]
    async fn deleting_an_account_invalidates_its_browser_sessions() {
        let repository = test_repository();
        let admin = repository
            .bootstrap_local_account(
                "11111111-1111-4111-8111-111111111111",
                "admin",
                "password123",
            )
            .await
            .expect("bootstrap should succeed");
        let user = repository
            .create_local_account("member", "password123", false)
            .await
            .expect("secondary account creation should succeed");
        repository
            .open_connection()
            .expect("opening the test database should succeed")
            .execute(
                "INSERT INTO browser_sessions (browser_session_id, user_id, created_at, last_seen_at, deleted_at)
                 VALUES (?1, ?2, ?3, ?4, NULL)",
                params![
                    "22222222-2222-4222-8222-222222222222",
                    user.user_id,
                    timestamp(&Utc::now()),
                    timestamp(&Utc::now())
                ],
            )
            .expect("binding a browser session should succeed");

        let invalidated = repository
            .delete_local_account(&user.user_id, &admin.user_id)
            .await
            .expect("deletion should succeed");
        let authenticated = repository
            .authenticate_browser_session("22222222-2222-4222-8222-222222222222")
            .await
            .expect("authentication lookup should succeed");

        assert_eq!(
            invalidated,
            vec!["22222222-2222-4222-8222-222222222222".to_string()]
        );
        assert!(authenticated.is_none());
    }

    #[tokio::test]
    async fn deleting_an_account_frees_the_username_for_reuse() {
        let repository = test_repository();
        let admin = repository
            .bootstrap_local_account(
                "11111111-1111-4111-8111-111111111111",
                "admin",
                "password123",
            )
            .await
            .expect("bootstrap should succeed");
        let original = repository
            .create_local_account("member", "password123", false)
            .await
            .expect("member creation should succeed");

        repository
            .delete_local_account(&original.user_id, &admin.user_id)
            .await
            .expect("deleting the member should succeed");
        let recreated = repository
            .create_local_account("member", "password123", false)
            .await
            .expect("recreating the username should succeed");
        let listed = repository
            .list_local_accounts()
            .await
            .expect("listing accounts should succeed");

        assert_ne!(recreated.user_id, original.user_id);
        assert_eq!(
            listed
                .iter()
                .filter(|account| account.username == "member")
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn admin_retention_rules_are_enforced_for_updates_and_deletes() {
        let repository = test_repository();
        let admin = repository
            .bootstrap_local_account(
                "11111111-1111-4111-8111-111111111111",
                "admin",
                "password123",
            )
            .await
            .expect("bootstrap should succeed");
        let user = repository
            .create_local_account("member", "password123", false)
            .await
            .expect("secondary account creation should succeed");

        let self_demotion = repository
            .update_local_account(&admin.user_id, &admin.user_id, None, Some(false))
            .await
            .expect_err("self demotion should fail");
        let last_admin_demotion = repository
            .update_local_account(&admin.user_id, &user.user_id, None, Some(false))
            .await
            .expect_err("demoting the last admin should fail");
        let self_delete = repository
            .delete_local_account(&admin.user_id, &admin.user_id)
            .await
            .expect_err("deleting the signed-in account should fail");
        let last_admin_delete = repository
            .delete_local_account(&admin.user_id, &user.user_id)
            .await
            .expect_err("deleting the last admin should fail");

        assert!(matches!(self_demotion, WorkspaceStoreError::Conflict(_)));
        assert!(matches!(
            last_admin_demotion,
            WorkspaceStoreError::Conflict(_)
        ));
        assert!(matches!(self_delete, WorkspaceStoreError::Conflict(_)));
        assert!(matches!(
            last_admin_delete,
            WorkspaceStoreError::Conflict(_)
        ));
    }

    #[tokio::test]
    async fn updating_an_account_rejects_blank_password_changes() {
        let repository = test_repository();
        let admin = repository
            .bootstrap_local_account(
                "11111111-1111-4111-8111-111111111111",
                "admin",
                "password123",
            )
            .await
            .expect("bootstrap should succeed");
        let member = repository
            .create_local_account("member", "password123", false)
            .await
            .expect("secondary account creation should succeed");

        let error = repository
            .update_local_account(&member.user_id, &admin.user_id, Some("   "), None)
            .await
            .expect_err("blank password changes should fail");

        assert_eq!(
            error,
            WorkspaceStoreError::Validation("password must not be empty".to_string())
        );
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
    async fn persist_session_snapshot_updates_activity_without_setting_terminal_timestamps() {
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
    }

    #[tokio::test]
    async fn persist_session_snapshot_records_close_transitions() {
        let repository = test_repository();
        let user = repository
            .materialize_user(&bearer_principal("developer"))
            .await
            .expect("principal materialization should succeed");
        let snapshot = snapshot("s_closed", "Transition", SessionStatus::Active);

        repository
            .persist_session_snapshot(&user.user_id, &snapshot, false, None)
            .await
            .expect("initial persistence should succeed");

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
    }

    #[tokio::test]
    async fn persist_session_snapshot_records_delete_transitions() {
        let repository = test_repository();
        let user = repository
            .materialize_user(&bearer_principal("developer"))
            .await
            .expect("principal materialization should succeed");
        let snapshot = snapshot("s_deleted", "Transition", SessionStatus::Active);

        repository
            .persist_session_snapshot(&user.user_id, &snapshot, false, None)
            .await
            .expect("initial persistence should succeed");

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
    fn workspace_store_accessors_preserve_context() {
        assert_eq!(WorkspaceStoreError::Io("io".to_string()).message(), "io");
        assert_eq!(
            WorkspaceStoreError::Unauthorized("auth".to_string()).message(),
            "auth"
        );
        assert_eq!(
            WorkspaceStoreError::NotFound("missing".to_string()).message(),
            "missing"
        );
        assert_eq!(
            WorkspaceStoreError::Conflict("conflict".to_string()).message(),
            "conflict"
        );
        assert_eq!(
            WorkspaceStoreError::Validation("invalid".to_string()).message(),
            "invalid"
        );
        assert_eq!(
            AuthenticatedPrincipalKind::BrowserSession.as_str(),
            "browser_session"
        );
    }

    #[test]
    fn ensure_parent_dir_accepts_parentless_paths() {
        assert!(ensure_parent_dir(Path::new("")).is_ok());
    }
}
