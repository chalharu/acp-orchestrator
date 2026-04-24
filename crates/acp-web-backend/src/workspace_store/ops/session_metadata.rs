use chrono::{DateTime, Utc};
use rusqlite::{Connection, params};

use crate::{
    contract_sessions::{SessionSnapshot, SessionStatus},
    workspace_records::{SessionMetadataRecord, WorkspaceRecord, WorkspaceStoreError},
};

use super::{
    ACTIVE_WORKSPACE_STATUS, BOOTSTRAP_WORKSPACE_KIND, BOOTSTRAP_WORKSPACE_NAME,
    queries::load_bootstrap_workspace,
    shared::{database_error, timestamp},
};

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
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL)
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

const UPDATE_SESSION_SNAPSHOT_PAYLOAD_SQL: &str = r#"
UPDATE sessions
SET
    latest_sequence = ?3,
    messages_json = ?4
WHERE owner_user_id = ?1 AND session_id = ?2
"#;

const INSERT_WORKSPACE_SQL: &str = r#"
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
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL)
"#;

const UPDATE_WORKSPACE_SQL: &str = r#"
UPDATE workspaces
SET
    name = ?3,
    default_ref = ?4,
    updated_at = ?5
WHERE owner_user_id = ?1 AND workspace_id = ?2 AND deleted_at IS NULL
"#;

const SOFT_DELETE_WORKSPACE_SQL: &str = r#"
UPDATE workspaces
SET
    status = 'deleted',
    updated_at = ?3,
    deleted_at = ?3
WHERE owner_user_id = ?1 AND workspace_id = ?2 AND deleted_at IS NULL
"#;

pub(in crate::workspace_store) fn bootstrap_workspace_in_transaction(
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
        upstream_url: None,
        default_ref: None,
        credential_reference_id: None,
        bootstrap_kind: Some(BOOTSTRAP_WORKSPACE_KIND.to_string()),
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
                workspace.upstream_url,
                workspace.default_ref,
                workspace.credential_reference_id,
                workspace.status,
                workspace.bootstrap_kind,
                timestamp(&workspace.created_at),
                timestamp(&workspace.updated_at)
            ],
        )
        .map_err(database_error)?;

    Ok(workspace)
}

pub(in crate::workspace_store) fn upsert_session_metadata(
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

pub(in crate::workspace_store) fn persist_session_snapshot_payload(
    connection: &Connection,
    owner_user_id: &str,
    snapshot: &SessionSnapshot,
) -> Result<(), WorkspaceStoreError> {
    let latest_sequence = i64::try_from(snapshot.latest_sequence)
        .map_err(|error| WorkspaceStoreError::Database(error.to_string()))?;
    let messages_json = serde_json::to_string(&snapshot.messages)
        .map_err(|error| WorkspaceStoreError::Database(error.to_string()))?;
    connection
        .execute(
            UPDATE_SESSION_SNAPSHOT_PAYLOAD_SQL,
            params![
                owner_user_id,
                snapshot.id.as_str(),
                latest_sequence,
                messages_json,
            ],
        )
        .map_err(database_error)?;
    Ok(())
}

pub(in crate::workspace_store) fn insert_workspace(
    connection: &Connection,
    record: &WorkspaceRecord,
) -> Result<(), WorkspaceStoreError> {
    connection
        .execute(
            INSERT_WORKSPACE_SQL,
            params![
                record.workspace_id,
                record.owner_user_id,
                record.name,
                record.upstream_url,
                record.default_ref,
                record.credential_reference_id,
                record.status,
                record.bootstrap_kind,
                timestamp(&record.created_at),
                timestamp(&record.updated_at),
            ],
        )
        .map_err(database_error)?;
    Ok(())
}

pub(in crate::workspace_store) fn update_workspace(
    connection: &Connection,
    owner_user_id: &str,
    workspace_id: &str,
    name: &str,
    default_ref: Option<&str>,
    updated_at: DateTime<Utc>,
) -> Result<bool, WorkspaceStoreError> {
    let affected = connection
        .execute(
            UPDATE_WORKSPACE_SQL,
            params![
                owner_user_id,
                workspace_id,
                name,
                default_ref,
                timestamp(&updated_at),
            ],
        )
        .map_err(database_error)?;
    Ok(affected != 0)
}

pub(in crate::workspace_store) fn soft_delete_workspace(
    connection: &Connection,
    owner_user_id: &str,
    workspace_id: &str,
    deleted_at: DateTime<Utc>,
) -> Result<bool, WorkspaceStoreError> {
    let affected = connection
        .execute(
            SOFT_DELETE_WORKSPACE_SQL,
            params![owner_user_id, workspace_id, timestamp(&deleted_at)],
        )
        .map_err(database_error)?;
    Ok(affected != 0)
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

pub(in crate::workspace_store) fn build_session_metadata_record(
    _connection: &Connection,
    owner_user_id: &str,
    snapshot: &SessionSnapshot,
    touch_activity: bool,
    status_override: Option<&str>,
    existing: Option<&SessionMetadataRecord>,
) -> Result<SessionMetadataRecord, WorkspaceStoreError> {
    let lifecycle = resolve_session_lifecycle(snapshot, touch_activity, status_override, existing);

    Ok(SessionMetadataRecord {
        session_id: snapshot.id.clone(),
        workspace_id: resolve_workspace_id(snapshot, existing)?,
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
    snapshot: &SessionSnapshot,
    existing: Option<&SessionMetadataRecord>,
) -> Result<String, WorkspaceStoreError> {
    if !snapshot.workspace_id.is_empty() {
        return Ok(snapshot.workspace_id.clone());
    }
    match existing {
        Some(record) => Ok(record.workspace_id.clone()),
        None => Err(WorkspaceStoreError::Validation(
            "session workspace_id must not be empty".to_string(),
        )),
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
