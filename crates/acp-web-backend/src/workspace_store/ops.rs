use std::{fmt, fmt::Write as _, path::Path};

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
use crate::workspace_records::{
    SessionMetadataRecord, UserRecord, WorkspaceRecord, WorkspaceStoreError,
};

const BOOTSTRAP_WORKSPACE_KIND: &str = "legacy-session-routes";
pub(super) const BOOTSTRAP_WORKSPACE_NAME: &str = "Default workspace";
const ACTIVE_WORKSPACE_STATUS: &str = "active";
pub(super) const LOCAL_ACCOUNT_PRINCIPAL_KIND: &str = "local_account";
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

pub(super) fn ensure_parent_dir(path: &Path) -> Result<(), WorkspaceStoreError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| WorkspaceStoreError::Io(format!("create state directory: {error}")))?;
    }
    Ok(())
}

pub(super) fn load_user_by_principal(
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

pub(super) fn load_user_by_id(
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

pub(super) fn load_active_local_account_by_username(
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

pub(super) fn load_session_metadata_record(
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

pub(super) fn open_immediate_transaction(
    connection: &mut Connection,
) -> Result<rusqlite::Transaction<'_>, WorkspaceStoreError> {
    connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(database_error)
}

pub(super) fn initialize_schema(connection: &Connection) -> Result<(), WorkspaceStoreError> {
    stage_legacy_browser_sessions_table(connection)?;
    connection
        .execute_batch(WORKSPACE_STORE_SCHEMA_SQL)
        .map_err(database_error)?;
    ensure_user_auth_columns(connection)?;
    migrate_legacy_auth_schema(connection)?;
    recreate_users_username_index(connection)?;
    Ok(())
}

fn ensure_user_auth_columns(connection: &Connection) -> Result<(), WorkspaceStoreError> {
    ensure_users_column(connection, "username", "TEXT")?;
    ensure_users_column(connection, "password_hash", "TEXT")?;
    ensure_users_column(connection, "is_admin", "INTEGER NOT NULL DEFAULT 0")?;
    ensure_users_column(connection, "deleted_at", "TEXT")?;
    Ok(())
}

fn migrate_legacy_auth_schema(connection: &Connection) -> Result<(), WorkspaceStoreError> {
    promote_legacy_bearer_admins(connection)?;
    migrate_legacy_local_accounts(connection)?;
    migrate_legacy_browser_sessions(connection)?;
    drop_legacy_auth_tables(connection)?;
    Ok(())
}

fn recreate_users_username_index(connection: &Connection) -> Result<(), WorkspaceStoreError> {
    connection
        .execute_batch(
            "DROP INDEX IF EXISTS users_username_idx;
                 CREATE UNIQUE INDEX IF NOT EXISTS users_username_idx
                    ON users(username)
                    WHERE username IS NOT NULL AND deleted_at IS NULL;",
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
    let accounts = query_legacy_local_accounts(connection, has_is_admin)?;
    Ok(Some((accounts, has_is_admin)))
}

fn query_legacy_local_accounts(
    connection: &Connection,
    has_is_admin: bool,
) -> Result<Vec<LegacyLocalAccountRecord>, WorkspaceStoreError> {
    let mut statement = connection
        .prepare(legacy_local_accounts_select_sql(has_is_admin))
        .map_err(database_error)?;
    statement
        .query_map([], legacy_local_account_from_row)
        .map_err(database_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error)
}

fn legacy_local_accounts_select_sql(has_is_admin: bool) -> &'static str {
    if has_is_admin {
        "SELECT user_name, password_hash, is_admin, created_at, updated_at
         FROM local_accounts
         ORDER BY created_at ASC, user_name ASC"
    } else {
        "SELECT user_name, password_hash, 0, created_at, updated_at
         FROM local_accounts
         ORDER BY created_at ASC, user_name ASC"
    }
}

fn legacy_local_account_from_row(
    row: &rusqlite::Row<'_>,
) -> Result<LegacyLocalAccountRecord, rusqlite::Error> {
    Ok(LegacyLocalAccountRecord {
        username: row.get(0)?,
        password_hash: row.get(1)?,
        is_admin: row.get::<_, i64>(2)? != 0,
        created_at: row.get(3)?,
        updated_at: row.get(4)?,
    })
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

pub(super) fn local_account_count(connection: &Connection) -> Result<i64, WorkspaceStoreError> {
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

pub(super) fn authenticate_browser_session(
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

pub(super) fn authenticate_browser_session_in_transaction(
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

pub(super) fn list_local_accounts(
    connection: &Connection,
) -> Result<Vec<LocalAccount>, WorkspaceStoreError> {
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

pub(super) fn validate_username(username: &str) -> Result<String, WorkspaceStoreError> {
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

pub(super) fn validate_password(password: &str) -> Result<(), WorkspaceStoreError> {
    if password.len() < 8 {
        return Err(WorkspaceStoreError::Validation(
            "password must be at least 8 characters".to_string(),
        ));
    }
    Ok(())
}

pub(super) fn hash_password(password: &str) -> Result<String, WorkspaceStoreError> {
    validate_password(password)?;
    let salt = encode_password_salt(Uuid::new_v4().as_bytes())?;
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|error| WorkspaceStoreError::Database(format!("failed to hash password: {error}")))
}

pub(super) fn encode_password_salt(bytes: &[u8]) -> Result<SaltString, WorkspaceStoreError> {
    SaltString::encode_b64(bytes)
        .map_err(|error| WorkspaceStoreError::Database(format!("failed to encode salt: {error}")))
}

pub(super) fn verify_password(
    password: &str,
    password_hash: &str,
) -> Result<bool, WorkspaceStoreError> {
    let parsed = PasswordHash::new(password_hash).map_err(|error| {
        WorkspaceStoreError::Database(format!("invalid password hash: {error}"))
    })?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}

pub(super) fn authenticate_local_account_in_transaction(
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

pub(super) fn materialize_browser_session_user_in_transaction(
    connection: &Connection,
    browser_session_id: &str,
) -> Result<UserRecord, WorkspaceStoreError> {
    authenticate_browser_session_in_transaction(connection, browser_session_id)?.ok_or_else(|| {
        WorkspaceStoreError::Unauthorized("browser session is not linked to an account".to_string())
    })
}

pub(super) fn materialize_bearer_user_in_transaction(
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

pub(super) fn insert_local_account(
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

pub(super) fn bind_browser_session_to_user(
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

pub(super) fn soft_delete_browser_session(
    connection: &Connection,
    browser_session_id: &str,
    now: DateTime<Utc>,
) -> Result<(), WorkspaceStoreError> {
    connection
        .execute(
            "UPDATE browser_sessions
             SET deleted_at = ?1
             WHERE browser_session_id = ?2 AND deleted_at IS NULL",
            params![timestamp(&now), browser_session_id],
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

pub(super) fn local_account_from_user(
    user: &UserRecord,
) -> Result<LocalAccount, WorkspaceStoreError> {
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

pub(super) fn update_local_account_in_transaction(
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

pub(super) fn delete_local_account_in_transaction(
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

pub(super) fn next_password_hash(
    password: Option<&str>,
) -> Result<Option<String>, WorkspaceStoreError> {
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

pub(super) fn durable_local_account_subject(username: &str) -> String {
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

pub(super) fn map_account_write_error(error: rusqlite::Error) -> WorkspaceStoreError {
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

pub(super) fn timestamp(value: &DateTime<Utc>) -> String {
    value.to_rfc3339()
}

pub(super) fn parse_timestamp(value: String) -> Result<DateTime<Utc>, WorkspaceStoreError> {
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

pub(super) fn parse_timestamp_for_row(
    value: String,
    index: usize,
) -> rusqlite::Result<DateTime<Utc>> {
    parse_timestamp(value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })
}

pub(super) fn parse_optional_timestamp_for_row(
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

pub(super) fn database_error(error: impl fmt::Display) -> WorkspaceStoreError {
    WorkspaceStoreError::Database(error.to_string())
}

pub(super) fn join_error(error: tokio::task::JoinError) -> WorkspaceStoreError {
    WorkspaceStoreError::Database(format!("blocking workspace task failed: {error}"))
}

impl AuthenticatedPrincipalKind {
    pub(super) fn as_str(&self) -> &'static str {
        match self {
            Self::Bearer => "bearer",
            Self::BrowserSession => "browser_session",
        }
    }
}

pub(super) fn durable_principal_subject(principal: &AuthenticatedPrincipal) -> String {
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

pub(super) fn bootstrap_workspace_in_transaction(
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

pub(super) fn upsert_session_metadata(
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

pub(super) fn build_session_metadata_record(
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
