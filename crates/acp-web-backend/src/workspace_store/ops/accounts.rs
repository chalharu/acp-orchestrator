use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

use crate::{
    auth::AuthenticatedPrincipal,
    contract_accounts::LocalAccount,
    workspace_records::{UserRecord, WorkspaceStoreError},
};

use super::{
    LOCAL_ACCOUNT_PRINCIPAL_KIND,
    queries::{
        load_active_local_account_by_username, load_user_by_id, load_user_by_principal,
        load_user_row,
    },
    shared::{
        database_error, durable_principal_subject, hash_subject, parse_timestamp_for_row, timestamp,
    },
};

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
    hash_subject(LOCAL_ACCOUNT_PRINCIPAL_KIND, username)
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
