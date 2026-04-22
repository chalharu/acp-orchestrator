use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};

use crate::workspace_records::{
    SessionMetadataRecord, UserRecord, WorkspaceRecord, WorkspaceStoreError,
};

use super::{
    BOOTSTRAP_WORKSPACE_KIND,
    shared::{database_error, parse_optional_timestamp_for_row, parse_timestamp_for_row},
};

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

pub(super) fn load_user_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<UserRecord> {
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

pub(super) fn load_bootstrap_workspace(
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
