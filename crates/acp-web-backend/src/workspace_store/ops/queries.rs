use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};

use crate::contract_sessions::{SessionSnapshot, SessionStatus};
use crate::workspace_records::{
    DurableSessionSnapshotRecord, SessionMetadataRecord, UserRecord, WorkspaceRecord,
    WorkspaceStoreError,
};

use super::{
    BOOTSTRAP_WORKSPACE_KIND,
    shared::{database_error, parse_optional_timestamp_for_row, parse_timestamp_for_row},
};

const LOAD_BOOTSTRAP_WORKSPACE_SQL: &str = r#"
SELECT
    workspace_id,
    owner_user_id,
    name,
    upstream_url,
    default_ref,
    credential_reference_id,
    bootstrap_kind,
    status,
    created_at,
    updated_at,
    deleted_at
FROM workspaces
WHERE owner_user_id = ?1 AND bootstrap_kind = ?2 AND deleted_at IS NULL
"#;

const LOAD_WORKSPACE_SQL: &str = r#"
SELECT
    workspace_id,
    owner_user_id,
    name,
    upstream_url,
    default_ref,
    credential_reference_id,
    bootstrap_kind,
    status,
    created_at,
    updated_at,
    deleted_at
FROM workspaces
WHERE owner_user_id = ?1 AND workspace_id = ?2 AND deleted_at IS NULL
"#;

const LIST_WORKSPACES_SQL: &str = r#"
SELECT
    workspace_id,
    owner_user_id,
    name,
    upstream_url,
    default_ref,
    credential_reference_id,
    bootstrap_kind,
    status,
    created_at,
    updated_at,
    deleted_at
FROM workspaces
WHERE owner_user_id = ?1 AND deleted_at IS NULL
ORDER BY
    CASE WHEN bootstrap_kind IS NULL THEN 1 ELSE 0 END,
    updated_at DESC,
    workspace_id ASC
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

const LIST_WORKSPACE_SESSIONS_SQL: &str = r#"
SELECT
    session_id,
    workspace_id,
    title,
    status,
    last_activity_at
FROM sessions
WHERE owner_user_id = ?1 AND workspace_id = ?2 AND deleted_at IS NULL
ORDER BY last_activity_at DESC, session_id ASC
"#;

const LOAD_SESSION_SNAPSHOT_SQL: &str = r#"
SELECT
    session_id,
    workspace_id,
    title,
    status,
    latest_sequence,
    messages_json,
    last_activity_at
FROM sessions
WHERE owner_user_id = ?1 AND session_id = ?2 AND deleted_at IS NULL
"#;

pub(in crate::workspace_store) fn load_user_by_principal(
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

pub(in crate::workspace_store) fn load_user_by_id(
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

pub(in crate::workspace_store) fn load_active_local_account_by_username(
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

pub(in crate::workspace_store) fn load_user_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<UserRecord> {
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

pub(in crate::workspace_store) fn load_bootstrap_workspace(
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

pub(in crate::workspace_store) fn list_workspaces(
    connection: &Connection,
    owner_user_id: &str,
) -> Result<Vec<WorkspaceRecord>, WorkspaceStoreError> {
    let mut statement = connection
        .prepare(LIST_WORKSPACES_SQL)
        .map_err(database_error)?;
    statement
        .query_map(params![owner_user_id], load_workspace_row)
        .map_err(database_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error)
}

pub(in crate::workspace_store) fn load_workspace(
    connection: &Connection,
    owner_user_id: &str,
    workspace_id: &str,
) -> Result<Option<WorkspaceRecord>, WorkspaceStoreError> {
    connection
        .query_row(
            LOAD_WORKSPACE_SQL,
            params![owner_user_id, workspace_id],
            load_workspace_row,
        )
        .optional()
        .map_err(database_error)
}

fn load_workspace_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkspaceRecord> {
    let identity = load_workspace_identity_fields(row)?;
    let repository = load_workspace_repository_fields(row)?;
    let timestamps = load_workspace_timestamp_fields(row)?;

    Ok(WorkspaceRecord {
        workspace_id: identity.workspace_id,
        owner_user_id: identity.owner_user_id,
        name: identity.name,
        upstream_url: repository.upstream_url,
        default_ref: repository.default_ref,
        credential_reference_id: repository.credential_reference_id,
        bootstrap_kind: repository.bootstrap_kind,
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
        status: row.get(7)?,
    })
}

struct WorkspaceRepositoryFields {
    upstream_url: Option<String>,
    default_ref: Option<String>,
    credential_reference_id: Option<String>,
    bootstrap_kind: Option<String>,
}

fn load_workspace_repository_fields(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<WorkspaceRepositoryFields> {
    Ok(WorkspaceRepositoryFields {
        upstream_url: row.get(3)?,
        default_ref: row.get(4)?,
        credential_reference_id: row.get(5)?,
        bootstrap_kind: row.get(6)?,
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
        created_at: parse_timestamp_for_row(row.get::<_, String>(8)?, 8)?,
        updated_at: parse_timestamp_for_row(row.get::<_, String>(9)?, 9)?,
        deleted_at: parse_optional_timestamp_for_row(row.get(10)?, 10)?,
    })
}

pub(in crate::workspace_store) fn load_session_metadata_record(
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

pub(in crate::workspace_store) fn load_session_snapshot_record(
    connection: &Connection,
    owner_user_id: &str,
    session_id: &str,
) -> Result<Option<DurableSessionSnapshotRecord>, WorkspaceStoreError> {
    connection
        .query_row(
            LOAD_SESSION_SNAPSHOT_SQL,
            params![owner_user_id, session_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, String>(5)?,
                    parse_timestamp_for_row(row.get::<_, String>(6)?, 6)?,
                ))
            },
        )
        .optional()
        .map_err(database_error)?
        .map(build_session_snapshot_record)
        .transpose()
}

pub(in crate::workspace_store) fn list_workspace_sessions(
    connection: &Connection,
    owner_user_id: &str,
    workspace_id: &str,
) -> Result<Vec<crate::contract_sessions::SessionListItem>, WorkspaceStoreError> {
    let mut statement = connection
        .prepare(LIST_WORKSPACE_SESSIONS_SQL)
        .map_err(database_error)?;
    statement
        .query_map(params![owner_user_id, workspace_id], |row| {
            Ok(crate::contract_sessions::SessionListItem {
                id: row.get(0)?,
                workspace_id: row.get(1)?,
                title: row.get(2)?,
                status: match row.get::<_, String>(3)?.as_str() {
                    "closed" => crate::contract_sessions::SessionStatus::Closed,
                    _ => crate::contract_sessions::SessionStatus::Active,
                },
                last_activity_at: parse_timestamp_for_row(row.get::<_, String>(4)?, 4)?,
            })
        })
        .map_err(database_error)?
        .collect::<Result<Vec<_>, _>>()
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

fn build_session_snapshot_record(
    row: (String, String, String, String, i64, String, DateTime<Utc>),
) -> Result<DurableSessionSnapshotRecord, WorkspaceStoreError> {
    let (session_id, workspace_id, title, status, latest_sequence, messages_json, last_activity_at) =
        row;
    let latest_sequence = u64::try_from(latest_sequence).map_err(|error| {
        WorkspaceStoreError::Database(format!(
            "invalid latest_sequence for session {session_id}: {error}"
        ))
    })?;
    let messages = serde_json::from_str(&messages_json).map_err(|error| {
        WorkspaceStoreError::Database(format!(
            "invalid messages_json for session {session_id}: {error}"
        ))
    })?;

    Ok(DurableSessionSnapshotRecord {
        session: SessionSnapshot {
            id: session_id,
            workspace_id,
            title,
            status: session_status_from_record(&status),
            latest_sequence,
            messages,
            pending_permissions: Vec::new(),
        },
        last_activity_at,
    })
}

fn session_status_from_record(status: &str) -> SessionStatus {
    match status {
        "closed" => SessionStatus::Closed,
        _ => SessionStatus::Active,
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
