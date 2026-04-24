use std::{path::PathBuf, sync::Arc, time::Duration};

use chrono::Utc;
#[cfg(test)]
use rusqlite::params;
use rusqlite::{Connection, TransactionBehavior};

#[cfg(test)]
use std::path::Path;

use crate::auth::{AuthenticatedPrincipal, AuthenticatedPrincipalKind};
use crate::contract_accounts::LocalAccount;
#[cfg(test)]
use crate::contract_sessions::SessionStatus;
use crate::contract_sessions::{SessionListItem, SessionSnapshot};
use crate::contract_workspaces::{CreateWorkspaceRequest, UpdateWorkspaceRequest};
pub use crate::workspace_records::{
    SessionMetadataRecord, UserRecord, WorkspaceRecord, WorkspaceStoreError,
};
use crate::workspace_repository::WorkspaceRepository;

mod ops;

#[cfg(test)]
use self::ops::{
    BOOTSTRAP_WORKSPACE_NAME, LOCAL_ACCOUNT_PRINCIPAL_KIND, durable_local_account_subject,
    durable_principal_subject, encode_password_salt, hash_password, map_account_write_error,
    next_password_hash, parse_optional_timestamp_for_row, parse_timestamp, parse_timestamp_for_row,
    timestamp, validate_password, validate_username, verify_password,
};
use self::ops::{
    authenticate_browser_session, authenticate_browser_session_in_transaction,
    authenticate_local_account_in_transaction, bind_browser_session_to_user,
    bootstrap_workspace_in_transaction, build_session_metadata_record, database_error,
    delete_local_account_in_transaction, ensure_parent_dir, initialize_schema,
    insert_local_account, insert_workspace, join_error, list_local_accounts,
    list_workspace_sessions, list_workspaces, load_session_metadata_record, load_workspace,
    local_account_count, local_account_from_user, materialize_bearer_user_in_transaction,
    materialize_browser_session_user_in_transaction, open_immediate_transaction,
    soft_delete_browser_session, soft_delete_workspace, update_local_account_in_transaction,
    update_workspace as update_workspace_record, upsert_session_metadata,
};

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
        let tx = open_immediate_transaction(&mut connection)?;
        initialize_schema(&tx)?;
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

    fn list_workspaces_sync(
        &self,
        owner_user_id: &str,
    ) -> Result<Vec<WorkspaceRecord>, WorkspaceStoreError> {
        let connection = self.open_connection()?;
        list_workspaces(&connection, owner_user_id)
    }

    fn load_workspace_sync(
        &self,
        owner_user_id: &str,
        workspace_id: &str,
    ) -> Result<Option<WorkspaceRecord>, WorkspaceStoreError> {
        let connection = self.open_connection()?;
        load_workspace(&connection, owner_user_id, workspace_id)
    }

    fn create_workspace_sync(
        &self,
        owner_user_id: &str,
        request: &CreateWorkspaceRequest,
    ) -> Result<WorkspaceRecord, WorkspaceStoreError> {
        let mut connection = self.open_connection()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        let workspace = build_workspace_record(owner_user_id, request)?;
        insert_workspace(&tx, &workspace)?;
        tx.commit().map_err(database_error)?;
        Ok(workspace)
    }

    fn update_workspace_sync(
        &self,
        owner_user_id: &str,
        workspace_id: &str,
        request: &UpdateWorkspaceRequest,
    ) -> Result<WorkspaceRecord, WorkspaceStoreError> {
        let mut connection = self.open_connection()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        let existing = load_workspace(&tx, owner_user_id, workspace_id)?
            .ok_or_else(|| WorkspaceStoreError::NotFound("workspace not found".to_string()))?;
        validate_workspace_update(&existing, request)?;
        let updated = WorkspaceRecord {
            name: request
                .name
                .as_deref()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .unwrap_or(existing.name.as_str())
                .to_string(),
            default_ref: request
                .default_ref
                .as_deref()
                .map(str::trim)
                .map(str::to_string)
                .or(existing.default_ref.clone()),
            updated_at: Utc::now(),
            ..existing
        };
        let updated_row = apply_workspace_update(&tx, owner_user_id, workspace_id, &updated)?;
        debug_assert!(
            updated_row,
            "loaded workspace should remain updateable within the transaction"
        );
        tx.commit().map_err(database_error)?;
        Ok(updated)
    }

    fn delete_workspace_sync(
        &self,
        owner_user_id: &str,
        workspace_id: &str,
    ) -> Result<(), WorkspaceStoreError> {
        let mut connection = self.open_connection()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        let workspace = load_workspace(&tx, owner_user_id, workspace_id)?
            .ok_or_else(|| WorkspaceStoreError::NotFound("workspace not found".to_string()))?;
        ensure_workspace_can_be_deleted(&tx, &workspace)?;
        let deleted = soft_delete_workspace(&tx, owner_user_id, workspace_id, Utc::now())?;
        debug_assert!(
            deleted,
            "loaded workspace should remain deletable within the transaction"
        );
        tx.commit().map_err(database_error)?;
        Ok(())
    }

    fn list_workspace_sessions_sync(
        &self,
        owner_user_id: &str,
        workspace_id: &str,
    ) -> Result<Vec<SessionListItem>, WorkspaceStoreError> {
        let connection = self.open_connection()?;
        ensure_workspace_exists(&connection, owner_user_id, workspace_id)?;
        list_workspace_sessions(&connection, owner_user_id, workspace_id)
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
        if !snapshot.workspace_id.is_empty() {
            ensure_workspace_exists(&tx, owner_user_id, &snapshot.workspace_id)?;
        }
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

    fn sign_out_browser_session_sync(
        &self,
        browser_session_id: &str,
    ) -> Result<(), WorkspaceStoreError> {
        let mut connection = self.open_connection()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        soft_delete_browser_session(&tx, browser_session_id, Utc::now())?;
        tx.commit().map_err(database_error)?;
        Ok(())
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

fn build_workspace_record(
    owner_user_id: &str,
    request: &CreateWorkspaceRequest,
) -> Result<WorkspaceRecord, WorkspaceStoreError> {
    let name = validate_workspace_name(&request.name)?;
    let upstream_url = validate_workspace_upstream_url(request.upstream_url.as_deref())?;
    let default_ref = validate_workspace_default_ref(request.default_ref.as_deref())?;
    let credential_reference_id =
        validate_credential_reference_id(request.credential_reference_id.as_deref())?;
    let now = Utc::now();

    Ok(WorkspaceRecord {
        workspace_id: format!("w_{}", uuid::Uuid::new_v4().simple()),
        owner_user_id: owner_user_id.to_string(),
        name,
        upstream_url,
        default_ref,
        credential_reference_id,
        bootstrap_kind: None,
        status: "active".to_string(),
        created_at: now,
        updated_at: now,
        deleted_at: None,
    })
}

fn validate_workspace_update(
    _existing: &WorkspaceRecord,
    request: &UpdateWorkspaceRequest,
) -> Result<(), WorkspaceStoreError> {
    if request.name.is_none() && request.default_ref.is_none() {
        return Err(WorkspaceStoreError::Validation(
            "workspace update must include name or default_ref".to_string(),
        ));
    }
    request
        .name
        .as_deref()
        .map(validate_workspace_name)
        .transpose()?;
    let _ = validate_workspace_default_ref(request.default_ref.as_deref())?;
    Ok(())
}

fn apply_workspace_update(
    connection: &Connection,
    owner_user_id: &str,
    workspace_id: &str,
    workspace: &WorkspaceRecord,
) -> Result<bool, WorkspaceStoreError> {
    update_workspace_record(
        connection,
        owner_user_id,
        workspace_id,
        &workspace.name,
        workspace.default_ref.as_deref(),
        workspace.updated_at,
    )
}

fn validate_workspace_name(name: &str) -> Result<String, WorkspaceStoreError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(WorkspaceStoreError::Validation(
            "workspace name must not be empty".to_string(),
        ));
    }
    if name.chars().count() > 120 {
        return Err(WorkspaceStoreError::Validation(
            "workspace name must not exceed 120 characters".to_string(),
        ));
    }
    Ok(name.to_string())
}

fn validate_workspace_upstream_url(
    upstream_url: Option<&str>,
) -> Result<Option<String>, WorkspaceStoreError> {
    let Some(upstream_url) = upstream_url else {
        return Ok(None);
    };
    let upstream_url = upstream_url.trim();
    if upstream_url.is_empty() {
        return Err(WorkspaceStoreError::Validation(
            "upstream_url must not be empty".to_string(),
        ));
    }
    let parsed = reqwest::Url::parse(upstream_url).map_err(|_| {
        WorkspaceStoreError::Validation("upstream_url must be a valid URL".to_string())
    })?;
    if parsed.scheme() != "https" {
        return Err(WorkspaceStoreError::Validation(
            "upstream_url must use https".to_string(),
        ));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(WorkspaceStoreError::Validation(
            "upstream_url must not embed credentials".to_string(),
        ));
    }
    Ok(Some(parsed.to_string()))
}

fn validate_workspace_default_ref(
    default_ref: Option<&str>,
) -> Result<Option<String>, WorkspaceStoreError> {
    let Some(default_ref) = default_ref else {
        return Ok(None);
    };
    let default_ref = default_ref.trim();
    if default_ref.is_empty() {
        return Err(WorkspaceStoreError::Validation(
            "default_ref must not be empty".to_string(),
        ));
    }
    if default_ref.chars().any(char::is_whitespace)
        || default_ref.ends_with('.')
        || default_ref.starts_with('/')
        || default_ref.ends_with('/')
        || default_ref.contains("..")
        || default_ref.contains('@')
        || default_ref.contains('\\')
    {
        return Err(WorkspaceStoreError::Validation(
            "default_ref is invalid".to_string(),
        ));
    }
    Ok(Some(default_ref.to_string()))
}

fn validate_credential_reference_id(
    credential_reference_id: Option<&str>,
) -> Result<Option<String>, WorkspaceStoreError> {
    let Some(credential_reference_id) = credential_reference_id else {
        return Ok(None);
    };
    let credential_reference_id = credential_reference_id.trim();
    if credential_reference_id.is_empty() {
        return Err(WorkspaceStoreError::Validation(
            "credential_reference_id must not be empty".to_string(),
        ));
    }
    Ok(Some(credential_reference_id.to_string()))
}

fn ensure_workspace_exists(
    connection: &Connection,
    owner_user_id: &str,
    workspace_id: &str,
) -> Result<(), WorkspaceStoreError> {
    load_workspace(connection, owner_user_id, workspace_id)?
        .ok_or_else(|| WorkspaceStoreError::NotFound("workspace not found".to_string()))?;
    Ok(())
}

fn ensure_workspace_can_be_deleted(
    connection: &Connection,
    workspace: &WorkspaceRecord,
) -> Result<(), WorkspaceStoreError> {
    let active_sessions = connection
        .query_row(
            "SELECT COUNT(1)
             FROM sessions
             WHERE owner_user_id = ?1
               AND workspace_id = ?2
               AND deleted_at IS NULL",
            rusqlite::params![workspace.owner_user_id, workspace.workspace_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(database_error)?;
    if active_sessions != 0 {
        return Err(WorkspaceStoreError::Conflict(
            "workspace_not_empty".to_string(),
        ));
    }
    Ok(())
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

    async fn list_workspaces(
        &self,
        owner_user_id: &str,
    ) -> Result<Vec<WorkspaceRecord>, WorkspaceStoreError> {
        let repository = self.clone();
        let owner_user_id = owner_user_id.to_string();
        tokio::task::spawn_blocking(move || repository.list_workspaces_sync(&owner_user_id))
            .await
            .map_err(join_error)?
    }

    async fn load_workspace(
        &self,
        owner_user_id: &str,
        workspace_id: &str,
    ) -> Result<Option<WorkspaceRecord>, WorkspaceStoreError> {
        let repository = self.clone();
        let owner_user_id = owner_user_id.to_string();
        let workspace_id = workspace_id.to_string();
        tokio::task::spawn_blocking(move || {
            repository.load_workspace_sync(&owner_user_id, &workspace_id)
        })
        .await
        .map_err(join_error)?
    }

    async fn create_workspace(
        &self,
        owner_user_id: &str,
        request: &CreateWorkspaceRequest,
    ) -> Result<WorkspaceRecord, WorkspaceStoreError> {
        let repository = self.clone();
        let owner_user_id = owner_user_id.to_string();
        let request = request.clone();
        tokio::task::spawn_blocking(move || {
            repository.create_workspace_sync(&owner_user_id, &request)
        })
        .await
        .map_err(join_error)?
    }

    async fn update_workspace(
        &self,
        owner_user_id: &str,
        workspace_id: &str,
        request: &UpdateWorkspaceRequest,
    ) -> Result<WorkspaceRecord, WorkspaceStoreError> {
        let repository = self.clone();
        let owner_user_id = owner_user_id.to_string();
        let workspace_id = workspace_id.to_string();
        let request = request.clone();
        tokio::task::spawn_blocking(move || {
            repository.update_workspace_sync(&owner_user_id, &workspace_id, &request)
        })
        .await
        .map_err(join_error)?
    }

    async fn delete_workspace(
        &self,
        owner_user_id: &str,
        workspace_id: &str,
    ) -> Result<(), WorkspaceStoreError> {
        let repository = self.clone();
        let owner_user_id = owner_user_id.to_string();
        let workspace_id = workspace_id.to_string();
        tokio::task::spawn_blocking(move || {
            repository.delete_workspace_sync(&owner_user_id, &workspace_id)
        })
        .await
        .map_err(join_error)?
    }

    async fn list_workspace_sessions(
        &self,
        owner_user_id: &str,
        workspace_id: &str,
    ) -> Result<Vec<SessionListItem>, WorkspaceStoreError> {
        let repository = self.clone();
        let owner_user_id = owner_user_id.to_string();
        let workspace_id = workspace_id.to_string();
        tokio::task::spawn_blocking(move || {
            repository.list_workspace_sessions_sync(&owner_user_id, &workspace_id)
        })
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

    async fn sign_out_browser_session(
        &self,
        browser_session_id: &str,
    ) -> Result<(), WorkspaceStoreError> {
        let repository = self.clone();
        let browser_session_id = browser_session_id.to_string();
        tokio::task::spawn_blocking(move || {
            repository.sign_out_browser_session_sync(&browser_session_id)
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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::sleep;

    mod workspaces;

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

    fn legacy_local_accounts_schema(with_admin_flags: bool) -> &'static str {
        if with_admin_flags {
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
                );"
        } else {
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
                );"
        }
    }

    fn legacy_local_accounts_db(
        label: &str,
        with_admin_flags: bool,
    ) -> (std::path::PathBuf, Connection) {
        legacy_user_db_connection(label, legacy_local_accounts_schema(with_admin_flags))
    }

    fn mixed_current_and_legacy_local_accounts_db() -> (std::path::PathBuf, Connection) {
        legacy_user_db_connection(
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
        )
    }

    fn legacy_timestamp(day: usize, hour: usize) -> String {
        format!("2024-01-{day:02}T{hour:02}:00:00Z")
    }

    fn insert_legacy_local_accounts_fixture(
        connection: &Connection,
        rows: &[(&str, &str, Option<bool>)],
    ) {
        for (index, (username, password, is_admin)) in rows.iter().enumerate() {
            let created_at = legacy_timestamp(index + 1, 0);
            let updated_at = legacy_timestamp(index + 1, 1);
            insert_legacy_local_account(
                connection,
                username,
                password,
                &created_at,
                &updated_at,
                *is_admin,
            );
        }
    }

    fn insert_legacy_browser_sessions_fixture(connection: &Connection, rows: &[(&str, &str)]) {
        for (index, (browser_session_id, username)) in rows.iter().enumerate() {
            let created_at = legacy_timestamp(index + 3, 0);
            let last_seen_at = legacy_timestamp(index + 3, 1);
            insert_legacy_browser_session(
                connection,
                browser_session_id,
                username,
                &created_at,
                &last_seen_at,
            );
        }
    }

    fn account_named<'a>(accounts: &'a [LocalAccount], username: &str) -> &'a LocalAccount {
        accounts
            .iter()
            .find(|account| account.username == username)
            .expect("migrated account should exist")
    }

    fn insert_current_local_account(connection: &Connection, username: &str, password: &str) {
        let password_hash = hash_password(password).expect("current password hash should build");
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
                    format!("u_current_{username}"),
                    LOCAL_ACCOUNT_PRINCIPAL_KIND,
                    durable_local_account_subject(username),
                    username,
                    password_hash,
                    now,
                ],
            )
            .expect("current local account should insert");
    }

    fn snapshot(
        workspace_id: &str,
        id: &str,
        title: &str,
        status: SessionStatus,
    ) -> SessionSnapshot {
        SessionSnapshot {
            id: id.to_string(),
            workspace_id: workspace_id.to_string(),
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
    async fn signing_out_invalidates_the_bound_browser_session() {
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
            .sign_out_browser_session("11111111-1111-4111-8111-111111111111")
            .await
            .expect("sign-out should succeed");

        let authenticated = repository
            .authenticate_browser_session("11111111-1111-4111-8111-111111111111")
            .await
            .expect("authentication lookup should succeed");
        let status = repository
            .auth_status(Some("11111111-1111-4111-8111-111111111111"))
            .await
            .expect("auth status should load");

        assert!(authenticated.is_none());
        assert!(!status.0);
        assert!(status.1.is_none());
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

    #[test]
    fn initialization_rejects_legacy_browser_session_table_name_collisions() {
        let (db_path, connection) = legacy_user_db_connection(
            "legacy-browser-session-table-collision",
            "CREATE TABLE browser_sessions (
                    session_token TEXT PRIMARY KEY,
                    principal_subject TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    last_seen_at TEXT NOT NULL
                );
                CREATE TABLE legacy_browser_sessions (
                    session_token TEXT PRIMARY KEY,
                    principal_subject TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    last_seen_at TEXT NOT NULL
                );",
        );
        drop(connection);

        let error = SqliteWorkspaceRepository::new(db_path)
            .expect_err("legacy table name collisions should fail initialization");

        assert_eq!(
            error,
            WorkspaceStoreError::Database(
                "legacy browser sessions table 'legacy_browser_sessions' already exists"
                    .to_string()
            )
        );
    }

    #[tokio::test]
    async fn initialization_migrates_legacy_local_accounts_and_browser_sessions_without_admin_flags()
     {
        let (db_path, connection) =
            legacy_local_accounts_db("legacy-local-accounts-no-admin", false);
        insert_legacy_local_accounts_fixture(
            &connection,
            &[("alice", "password123", None), ("bob", "password456", None)],
        );
        insert_legacy_browser_sessions_fixture(
            &connection,
            &[
                ("legacy-session-alice", "alice"),
                ("legacy-session-missing", "missing"),
            ],
        );
        drop(connection);

        let repository =
            SqliteWorkspaceRepository::new(db_path).expect("repository should migrate");
        let accounts = repository
            .list_local_accounts()
            .await
            .expect("listing migrated accounts should succeed");
        let alice = account_named(&accounts, "alice");
        let bob = account_named(&accounts, "bob");
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
        let (db_path, connection) =
            legacy_local_accounts_db("legacy-local-accounts-with-admin", true);
        insert_legacy_local_accounts_fixture(
            &connection,
            &[
                ("alice", "password123", Some(false)),
                ("bob", "password456", Some(true)),
            ],
        );
        insert_legacy_browser_sessions_fixture(&connection, &[("legacy-session-bob", "bob")]);
        drop(connection);

        let repository =
            SqliteWorkspaceRepository::new(db_path).expect("repository should migrate");
        let accounts = repository
            .list_local_accounts()
            .await
            .expect("listing migrated accounts should succeed");
        let alice = account_named(&accounts, "alice");
        let bob = account_named(&accounts, "bob");
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
        let (db_path, connection) = mixed_current_and_legacy_local_accounts_db();
        insert_current_local_account(&connection, "alice", "password-new");
        insert_legacy_local_accounts_fixture(&connection, &[("alice", "password-old", Some(true))]);
        insert_legacy_browser_sessions_fixture(&connection, &[("legacy-session-alice", "alice")]);
        drop(connection);

        let repository =
            SqliteWorkspaceRepository::new(db_path).expect("repository should migrate");
        let accounts = repository
            .list_local_accounts()
            .await
            .expect("listing migrated accounts should succeed");
        let alice = account_named(&accounts, "alice");
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
        let workspace = repository
            .bootstrap_workspace(&user.user_id)
            .await
            .expect("workspace bootstrap should succeed");
        let snapshot = snapshot(
            &workspace.workspace_id,
            "s_persisted",
            "Initial title",
            SessionStatus::Active,
        );

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
        let workspace = repository
            .bootstrap_workspace(&user.user_id)
            .await
            .expect("workspace bootstrap should succeed");
        let snapshot = snapshot(
            &workspace.workspace_id,
            "s_transition",
            "Transition",
            SessionStatus::Active,
        );

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
        let workspace = repository
            .bootstrap_workspace(&user.user_id)
            .await
            .expect("workspace bootstrap should succeed");
        let snapshot = snapshot(
            &workspace.workspace_id,
            "s_closed",
            "Transition",
            SessionStatus::Active,
        );

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
        let workspace = repository
            .bootstrap_workspace(&user.user_id)
            .await
            .expect("workspace bootstrap should succeed");
        let snapshot = snapshot(
            &workspace.workspace_id,
            "s_deleted",
            "Transition",
            SessionStatus::Active,
        );

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
        let workspace = repository
            .bootstrap_workspace(&user.user_id)
            .await
            .expect("workspace bootstrap should succeed");
        repository
            .open_connection()
            .expect("opening the test database should succeed")
            .execute("DROP TABLE workspaces", [])
            .expect("dropping workspaces should succeed");

        let error = repository
            .persist_session_snapshot(
                &user.user_id,
                &snapshot(
                    &workspace.workspace_id,
                    "s_broken",
                    "Broken",
                    SessionStatus::Active,
                ),
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
