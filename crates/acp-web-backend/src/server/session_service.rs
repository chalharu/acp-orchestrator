use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use chrono::Utc;

use crate::{
    auth::{AuthenticatedPrincipal, AuthenticatedPrincipalKind},
    contract_sessions::SessionSnapshot,
    mock_client::{ReplyProvider, ReplyResult},
    sessions::{PendingPrompt, SessionStoreError},
    workspace_records::UserRecord,
    workspace_records::{SessionMetadataRecord, WorkspaceRecord},
    workspace_repository::NewWorkspace,
};

use super::{AppError, AppState};

#[derive(Debug, Clone)]
pub(super) struct LiveSessionWriteContext {
    pub(super) principal: AuthenticatedPrincipal,
    pub(super) user: Option<UserRecord>,
}

pub(super) async fn create_session_snapshot(
    state: &AppState,
    principal: AuthenticatedPrincipal,
    checkout_ref_override: Option<String>,
) -> Result<SessionSnapshot, AppError> {
    let owner = state.owner_context(principal).await?;
    let workspaces = state
        .workspace_repository
        .list_workspaces(&owner.user.user_id)
        .await?;
    let workspace_id = match workspaces.as_slice() {
        [workspace] => workspace.workspace_id.clone(),
        [] => {
            state
                .workspace_repository
                .create_workspace(&owner.user.user_id, &legacy_session_workspace())
                .await?
                .workspace_id
        }
        _ => {
            return Err(AppError::Conflict(
                "workspace selection required".to_string(),
            ));
        }
    };

    create_session_snapshot_in_workspace(state, owner, &workspace_id, checkout_ref_override).await
}

fn legacy_session_workspace() -> NewWorkspace {
    NewWorkspace {
        name: "Workspace".to_string(),
        upstream_url: None,
        default_ref: None,
        credential_reference_id: None,
    }
}

pub(super) async fn create_session_snapshot_for_workspace(
    state: &AppState,
    principal: AuthenticatedPrincipal,
    workspace_id: &str,
    checkout_ref_override: Option<String>,
) -> Result<SessionSnapshot, AppError> {
    let owner = state.owner_context(principal).await?;
    create_session_snapshot_in_workspace(state, owner, workspace_id, checkout_ref_override).await
}

async fn create_session_snapshot_in_workspace(
    state: &AppState,
    owner: super::OwnerContext,
    workspace_id: &str,
    checkout_ref_override: Option<String>,
) -> Result<SessionSnapshot, AppError> {
    let workspace = load_workspace_for_session(state, &owner.user.user_id, workspace_id).await?;
    let session = state
        .store
        .create_session(&owner.principal.id, workspace_id)
        .await?;
    persist_provisioning_session_lifecycle(state, &owner, &workspace, &session).await?;
    let (session, checkout) = prepare_session_startup(
        state,
        &owner.principal.id,
        &owner.user,
        &workspace,
        session,
        checkout_ref_override.as_deref(),
    )
    .await?;
    persist_started_session_metadata(state, &owner, &workspace, &session, &checkout).await?;
    Ok(session)
}

async fn load_workspace_for_session(
    state: &AppState,
    user_id: &str,
    workspace_id: &str,
) -> Result<WorkspaceRecord, AppError> {
    state
        .workspace_repository
        .load_workspace(user_id, workspace_id)
        .await?
        .ok_or_else(|| AppError::NotFound("workspace not found".to_string()))
}

async fn persist_provisioning_session_lifecycle(
    state: &AppState,
    owner: &super::OwnerContext,
    workspace: &WorkspaceRecord,
    session: &SessionSnapshot,
) -> Result<(), AppError> {
    if let Err(error) = persist_session_lifecycle(
        state,
        &owner.user,
        workspace,
        session,
        "provisioning",
        None,
        None,
    )
    .await
    {
        rollback_session_creation(state, &owner.principal.id, &session.id, error.message()).await?;
        return Err(error);
    }
    Ok(())
}

async fn persist_started_session_metadata(
    state: &AppState,
    owner: &super::OwnerContext,
    workspace: &WorkspaceRecord,
    session: &SessionSnapshot,
    checkout: &crate::workspace_checkout::PreparedWorkspaceCheckout,
) -> Result<(), AppError> {
    if let Err(error) = persist_session_metadata(state, &owner.user, session, true, None).await {
        cleanup_checkout_path_best_effort(&checkout.working_dir);
        persist_failed_session_lifecycle(
            state,
            &owner.user,
            workspace,
            session,
            Some(checkout),
            error.message(),
        )
        .await;
        rollback_session_creation(state, &owner.principal.id, &session.id, error.message()).await?;
        return Err(error);
    }
    Ok(())
}

pub(super) async fn rename_session_title(
    state: &AppState,
    principal: AuthenticatedPrincipal,
    session_id: &str,
    title: String,
) -> Result<SessionSnapshot, AppError> {
    let owner = live_session_write_context(state, principal, "rename").await?;
    let title = title.trim().to_string();
    if title.is_empty() {
        return Err(AppError::BadRequest("title must not be empty".to_string()));
    }
    if title.chars().count() > 500 {
        return Err(AppError::BadRequest(
            "title must not exceed 500 characters".to_string(),
        ));
    }

    let session = state
        .store
        .rename_session(&owner.principal.id, session_id, title)
        .await?;
    if let Some(user) = owner.user.as_ref() {
        persist_session_metadata_best_effort(state, user, &session, false, None, "rename").await;
    }

    Ok(session)
}

pub(super) async fn delete_live_session(
    state: &AppState,
    principal: AuthenticatedPrincipal,
    session_id: &str,
) -> Result<(), AppError> {
    let owner = live_session_write_context(state, principal, "delete").await?;
    let checkout_cleanup_path = match owner.user.as_ref() {
        Some(user) => {
            load_checkout_cleanup_path_best_effort(state, user, session_id, "delete").await
        }
        None => None,
    };
    let snapshot = state
        .store
        .session_snapshot(&owner.principal.id, session_id)
        .await?;
    state
        .store
        .delete_session(&owner.principal.id, session_id)
        .await?;
    state.reply_provider.forget_session(session_id);
    if let Some(user) = owner.user.as_ref() {
        persist_session_metadata_best_effort(
            state,
            user,
            &snapshot,
            false,
            Some("deleted"),
            "delete",
        )
        .await;
    }
    if let Some(path) = checkout_cleanup_path.as_deref() {
        cleanup_checkout_path_best_effort(path);
    }

    Ok(())
}

pub(super) async fn submit_prompt(
    state: &AppState,
    principal: AuthenticatedPrincipal,
    session_id: &str,
    text: String,
) -> Result<(), AppError> {
    let owner = live_session_write_context(state, principal, "submit_prompt").await?;
    let pending = state
        .store
        .submit_prompt(&owner.principal.id, session_id, text)
        .await?;
    let snapshot_result = state
        .store
        .session_snapshot(&owner.principal.id, session_id)
        .await;
    if let Some(user) = owner.user.as_ref() {
        persist_prompt_snapshot_best_effort(state, user, session_id, snapshot_result).await;
    }
    dispatch_assistant_request(
        state.clone(),
        state.reply_provider.clone(),
        owner.principal.id.clone(),
        owner.user,
        pending,
    );

    Ok(())
}

pub(super) async fn close_live_session(
    state: &AppState,
    principal: AuthenticatedPrincipal,
    session_id: &str,
) -> Result<SessionSnapshot, AppError> {
    let owner = live_session_write_context(state, principal, "close").await?;
    let session = state
        .store
        .close_session(&owner.principal.id, session_id)
        .await?;
    state.reply_provider.forget_session(session_id);
    if let Some(user) = owner.user.as_ref() {
        persist_session_metadata_best_effort(state, user, &session, false, Some("closed"), "close")
            .await;
    }

    Ok(session)
}

async fn persist_session_metadata(
    state: &AppState,
    user: &UserRecord,
    snapshot: &SessionSnapshot,
    touch_activity: bool,
    status_override: Option<&str>,
) -> Result<(), AppError> {
    state
        .workspace_repository
        .persist_session_snapshot(&user.user_id, snapshot, touch_activity, status_override)
        .await
        .map_err(AppError::from)?;
    Ok(())
}

pub(super) async fn persist_session_metadata_best_effort(
    state: &AppState,
    user: &UserRecord,
    snapshot: &SessionSnapshot,
    touch_activity: bool,
    status_override: Option<&str>,
    action: &'static str,
) {
    if let Err(error) =
        persist_session_metadata(state, user, snapshot, touch_activity, status_override).await
    {
        let error_message = error.message();
        tracing::warn!(
            session_id = %snapshot.id,
            owner_user_id = %user.user_id,
            action,
            "failed to persist session metadata: {error_message}"
        );
    }
}

async fn materialize_user_best_effort(
    state: &AppState,
    principal: &AuthenticatedPrincipal,
    action: &'static str,
) -> Option<UserRecord> {
    match state.workspace_repository.materialize_user(principal).await {
        Ok(user) => Some(user),
        Err(error) => {
            let error = AppError::from(error);
            let error_message = error.message();
            tracing::warn!(
                %error_message,
                principal_kind = ?principal.kind,
                action,
                "failed to materialize durable user"
            );
            None
        }
    }
}

async fn live_session_write_context(
    state: &AppState,
    principal: AuthenticatedPrincipal,
    action: &'static str,
) -> Result<LiveSessionWriteContext, AppError> {
    match principal.kind {
        AuthenticatedPrincipalKind::Bearer => Ok(LiveSessionWriteContext {
            user: materialize_user_best_effort(state, &principal, action).await,
            principal,
        }),
        AuthenticatedPrincipalKind::BrowserSession => {
            let owner = state.owner_context(principal).await?;
            Ok(LiveSessionWriteContext {
                principal: owner.principal,
                user: Some(owner.user),
            })
        }
    }
}

pub(super) async fn persist_prompt_snapshot_best_effort(
    state: &AppState,
    user: &UserRecord,
    session_id: &str,
    snapshot_result: Result<SessionSnapshot, SessionStoreError>,
) {
    persist_snapshot_result_best_effort(state, user, session_id, snapshot_result, "submit_prompt")
        .await;
}

async fn persist_snapshot_result_best_effort(
    state: &AppState,
    user: &UserRecord,
    session_id: &str,
    snapshot_result: Result<SessionSnapshot, SessionStoreError>,
    action: &'static str,
) {
    match snapshot_result {
        Ok(snapshot) => {
            persist_session_metadata_best_effort(state, user, &snapshot, true, None, action).await;
        }
        Err(error) => {
            let error_message = error.message();
            tracing::warn!(
                session_id = %session_id,
                action,
                "failed to snapshot session metadata after action: {error_message}"
            );
        }
    }
}

// Session creation eagerly primes ACP so checkout/startup failures surface before the
// session is reported active. When startup hints are enabled, the priming output is
// also appended to the transcript as the first assistant message.
async fn prime_session_startup(
    state: &AppState,
    owner: &str,
    session: &SessionSnapshot,
) -> Result<SessionSnapshot, AppError> {
    let Some(hint) = state
        .reply_provider
        .prime_session(&session.id)
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?
    else {
        return Ok(session.clone());
    };
    if !state.startup_hints {
        return Ok(session.clone());
    }

    state
        .store
        .append_assistant_message(owner, &session.id, hint)
        .await
        .map_err(AppError::from)
}

async fn prepare_session_startup(
    state: &AppState,
    live_owner_id: &str,
    user: &UserRecord,
    workspace: &WorkspaceRecord,
    session: SessionSnapshot,
    checkout_ref_override: Option<&str>,
) -> Result<
    (
        SessionSnapshot,
        crate::workspace_checkout::PreparedWorkspaceCheckout,
    ),
    AppError,
> {
    persist_cloning_session_lifecycle(state, live_owner_id, user, workspace, &session).await?;
    let checkout = prepare_workspace_checkout(
        state,
        live_owner_id,
        user,
        workspace,
        &session,
        checkout_ref_override,
    )
    .await?;
    persist_starting_session_lifecycle(state, live_owner_id, user, workspace, &session, &checkout)
        .await?;
    bind_session_checkout(state, live_owner_id, user, workspace, &session, &checkout).await?;
    let session = prime_session_startup_or_rollback(
        state,
        live_owner_id,
        user,
        workspace,
        &session,
        &checkout,
    )
    .await?;
    Ok((session, checkout))
}

async fn persist_cloning_session_lifecycle(
    state: &AppState,
    live_owner_id: &str,
    user: &UserRecord,
    workspace: &WorkspaceRecord,
    session: &SessionSnapshot,
) -> Result<(), AppError> {
    if let Err(error) =
        persist_session_lifecycle(state, user, workspace, session, "cloning", None, None).await
    {
        persist_failed_session_lifecycle(state, user, workspace, session, None, error.message())
            .await;
        rollback_session_creation(state, live_owner_id, &session.id, error.message()).await?;
        return Err(error);
    }
    Ok(())
}

async fn prepare_workspace_checkout(
    state: &AppState,
    live_owner_id: &str,
    user: &UserRecord,
    workspace: &WorkspaceRecord,
    session: &SessionSnapshot,
    checkout_ref_override: Option<&str>,
) -> Result<crate::workspace_checkout::PreparedWorkspaceCheckout, AppError> {
    match state
        .checkout_manager
        .prepare_checkout(workspace, &session.id, checkout_ref_override)
        .await
    {
        Ok(checkout) => Ok(checkout),
        Err(error) => {
            persist_failed_session_lifecycle(
                state,
                user,
                workspace,
                session,
                None,
                error.message(),
            )
            .await;
            rollback_session_creation(state, live_owner_id, &session.id, error.message()).await?;
            Err(map_checkout_error(error))
        }
    }
}

async fn persist_starting_session_lifecycle(
    state: &AppState,
    live_owner_id: &str,
    user: &UserRecord,
    workspace: &WorkspaceRecord,
    session: &SessionSnapshot,
    checkout: &crate::workspace_checkout::PreparedWorkspaceCheckout,
) -> Result<(), AppError> {
    if let Err(error) = persist_session_lifecycle(
        state,
        user,
        workspace,
        session,
        "starting",
        Some(checkout),
        None,
    )
    .await
    {
        cleanup_checkout_path_best_effort(&checkout.working_dir);
        persist_failed_session_lifecycle(
            state,
            user,
            workspace,
            session,
            Some(checkout),
            error.message(),
        )
        .await;
        rollback_session_creation(state, live_owner_id, &session.id, error.message()).await?;
        return Err(error);
    }
    Ok(())
}

async fn bind_session_checkout(
    state: &AppState,
    live_owner_id: &str,
    user: &UserRecord,
    workspace: &WorkspaceRecord,
    session: &SessionSnapshot,
    checkout: &crate::workspace_checkout::PreparedWorkspaceCheckout,
) -> Result<(), AppError> {
    if let Err(error) = state
        .reply_provider
        .bind_session(&session.id, checkout.working_dir.clone())
        .await
    {
        cleanup_checkout_path_best_effort(&checkout.working_dir);
        persist_failed_session_lifecycle(state, user, workspace, session, Some(checkout), &error)
            .await;
        rollback_session_creation(state, live_owner_id, &session.id, &error).await?;
        return Err(AppError::Internal(error));
    }
    Ok(())
}

async fn prime_session_startup_or_rollback(
    state: &AppState,
    live_owner_id: &str,
    user: &UserRecord,
    workspace: &WorkspaceRecord,
    session: &SessionSnapshot,
    checkout: &crate::workspace_checkout::PreparedWorkspaceCheckout,
) -> Result<SessionSnapshot, AppError> {
    match prime_session_startup(state, live_owner_id, session).await {
        Ok(session) => Ok(session),
        Err(error) => {
            cleanup_checkout_path_best_effort(&checkout.working_dir);
            persist_failed_session_lifecycle(
                state,
                user,
                workspace,
                session,
                Some(checkout),
                error.message(),
            )
            .await;
            rollback_session_creation(state, live_owner_id, &session.id, error.message()).await?;
            Err(error)
        }
    }
}

async fn persist_session_lifecycle(
    state: &AppState,
    user: &UserRecord,
    workspace: &WorkspaceRecord,
    snapshot: &SessionSnapshot,
    status: &str,
    checkout: Option<&crate::workspace_checkout::PreparedWorkspaceCheckout>,
    failure_reason: Option<&str>,
) -> Result<(), AppError> {
    let existing = state
        .workspace_repository
        .load_session_metadata(&user.user_id, &snapshot.id)
        .await?;
    let record = build_session_lifecycle_record(
        existing.as_ref(),
        user,
        workspace,
        snapshot,
        status,
        checkout,
        failure_reason,
    );
    state
        .workspace_repository
        .save_session_metadata(&record)
        .await
        .map_err(AppError::from)
}

fn build_session_lifecycle_record(
    existing: Option<&SessionMetadataRecord>,
    user: &UserRecord,
    workspace: &WorkspaceRecord,
    snapshot: &SessionSnapshot,
    status: &str,
    checkout: Option<&crate::workspace_checkout::PreparedWorkspaceCheckout>,
    failure_reason: Option<&str>,
) -> SessionMetadataRecord {
    let now = Utc::now();
    let created_at = existing.map(|record| record.created_at).unwrap_or(now);
    SessionMetadataRecord {
        session_id: snapshot.id.clone(),
        workspace_id: workspace.workspace_id.clone(),
        owner_user_id: user.user_id.clone(),
        title: snapshot.title.clone(),
        status: status.to_string(),
        checkout_relpath: merged_checkout_relpath(existing, checkout),
        checkout_ref: merged_checkout_ref(existing, checkout),
        checkout_commit_sha: merged_checkout_commit_sha(existing, checkout),
        failure_reason: merged_failure_reason(existing, failure_reason),
        detach_deadline_at: existing.and_then(|record| record.detach_deadline_at),
        restartable_deadline_at: existing.and_then(|record| record.restartable_deadline_at),
        created_at,
        last_activity_at: existing
            .map(|record| record.last_activity_at)
            .unwrap_or(created_at),
        closed_at: existing.and_then(|record| record.closed_at),
        deleted_at: existing.and_then(|record| record.deleted_at),
    }
}

fn merged_checkout_relpath(
    existing: Option<&SessionMetadataRecord>,
    checkout: Option<&crate::workspace_checkout::PreparedWorkspaceCheckout>,
) -> Option<String> {
    checkout
        .map(|prepared| prepared.checkout_relpath.clone())
        .or_else(|| existing.and_then(|record| record.checkout_relpath.clone()))
}

fn merged_checkout_ref(
    existing: Option<&SessionMetadataRecord>,
    checkout: Option<&crate::workspace_checkout::PreparedWorkspaceCheckout>,
) -> Option<String> {
    checkout
        .and_then(|prepared| prepared.checkout_ref.clone())
        .or_else(|| existing.and_then(|record| record.checkout_ref.clone()))
}

fn merged_checkout_commit_sha(
    existing: Option<&SessionMetadataRecord>,
    checkout: Option<&crate::workspace_checkout::PreparedWorkspaceCheckout>,
) -> Option<String> {
    checkout
        .and_then(|prepared| prepared.checkout_commit_sha.clone())
        .or_else(|| existing.and_then(|record| record.checkout_commit_sha.clone()))
}

fn merged_failure_reason(
    existing: Option<&SessionMetadataRecord>,
    failure_reason: Option<&str>,
) -> Option<String> {
    failure_reason
        .map(str::to_string)
        .or_else(|| existing.and_then(|record| record.failure_reason.clone()))
}

async fn persist_failed_session_lifecycle(
    state: &AppState,
    user: &UserRecord,
    workspace: &WorkspaceRecord,
    snapshot: &SessionSnapshot,
    checkout: Option<&crate::workspace_checkout::PreparedWorkspaceCheckout>,
    failure_reason: &str,
) {
    if let Err(persist_error) = persist_session_lifecycle(
        state,
        user,
        workspace,
        snapshot,
        "failed",
        checkout,
        Some(failure_reason),
    )
    .await
    {
        tracing::warn!(
            session_id = %snapshot.id,
            owner_user_id = %user.user_id,
            error = %persist_error.message(),
            "failed to persist failed session lifecycle"
        );
    }
}

async fn load_checkout_cleanup_path_best_effort(
    state: &AppState,
    user: &UserRecord,
    session_id: &str,
    action: &'static str,
) -> Option<PathBuf> {
    match state
        .workspace_repository
        .load_session_metadata(&user.user_id, session_id)
        .await
    {
        Ok(Some(metadata)) => match metadata.checkout_relpath.as_deref() {
            Some(checkout_relpath) => match state
                .checkout_manager
                .resolve_checkout_path(checkout_relpath)
            {
                Some(path) => Some(path),
                None => {
                    tracing::warn!(
                        session_id = %session_id,
                        checkout_relpath,
                        action,
                        "persisted checkout path was invalid"
                    );
                    None
                }
            },
            None => None,
        },
        Ok(None) => None,
        Err(error) => {
            tracing::warn!(
                session_id = %session_id,
                action,
                "failed to load session metadata for checkout cleanup: {}",
                error.message()
            );
            None
        }
    }
}

fn cleanup_checkout_path_best_effort(path: &Path) {
    if let Err(error) = std::fs::remove_dir_all(path)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(path = %path.display(), "failed to remove session checkout: {error}");
    }
}

fn map_checkout_error(error: crate::workspace_checkout::WorkspaceCheckoutError) -> AppError {
    match error {
        crate::workspace_checkout::WorkspaceCheckoutError::Validation(message) => {
            AppError::BadRequest(message)
        }
        crate::workspace_checkout::WorkspaceCheckoutError::Io(message)
        | crate::workspace_checkout::WorkspaceCheckoutError::Git(message) => {
            tracing::error!(checkout_error = %message, "workspace checkout failed");
            AppError::Internal("checkout preparation failed".to_string())
        }
    }
}

async fn rollback_failed_session(
    state: &AppState,
    owner: &str,
    session_id: &str,
) -> Result<(), AppError> {
    state.reply_provider.forget_session(session_id);
    state
        .store
        .discard_session(owner, session_id)
        .await
        .map_err(|error| AppError::Internal(error.message().to_string()))
}

async fn rollback_session_creation(
    state: &AppState,
    owner_id: &str,
    session_id: &str,
    error_message: &str,
) -> Result<(), AppError> {
    rollback_failed_session(state, owner_id, session_id)
        .await
        .map_err(|rollback_error| {
            AppError::Internal(format!(
                "{error_message}; session rollback failed: {}",
                rollback_error.message()
            ))
        })
}

fn dispatch_assistant_request(
    state: AppState,
    reply_provider: Arc<dyn ReplyProvider>,
    live_owner_id: String,
    durable_user: Option<UserRecord>,
    pending: PendingPrompt,
) {
    tokio::spawn(async move {
        let session_id = pending.session_id().to_string();
        match reply_provider.request_reply(pending.turn_handle()).await {
            Ok(ReplyResult::Reply(reply)) => pending.complete_with_reply(reply).await,
            Ok(ReplyResult::Status(message)) => pending.complete_with_status(message).await,
            Ok(ReplyResult::NoOutput) => pending.complete_without_output().await,
            Err(error) => {
                pending
                    .complete_with_status(format!("ACP request failed: {error}"))
                    .await;
            }
        }

        if let Some(user) = durable_user.as_ref() {
            let snapshot_result = state
                .store
                .session_snapshot(&live_owner_id, &session_id)
                .await;
            persist_snapshot_result_best_effort(
                &state,
                user,
                &session_id,
                snapshot_result,
                "complete_prompt",
            )
            .await;
        }
    });
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;

    use super::*;
    use crate::{
        contract_accounts::LocalAccount,
        contract_sessions::SessionStatus,
        mock_client::ReplyFuture,
        workspace_checkout::{PreparedWorkspaceCheckout, WorkspaceCheckoutManager},
        workspace_records::{DurableSessionSnapshotRecord, WorkspaceStoreError},
        workspace_repository::{NewWorkspace, WorkspaceRepository, WorkspaceUpdatePatch},
    };

    #[derive(Debug)]
    struct NoopReplyProvider;

    impl ReplyProvider for NoopReplyProvider {
        fn request_reply<'a>(&'a self, _turn: crate::sessions::TurnHandle) -> ReplyFuture<'a> {
            Box::pin(async { Ok(ReplyResult::NoOutput) })
        }
    }

    #[derive(Debug)]
    struct StubWorkspaceRepository {
        metadata: Option<SessionMetadataRecord>,
        load_error: Option<WorkspaceStoreError>,
        save_error: Option<WorkspaceStoreError>,
    }

    #[async_trait]
    impl WorkspaceRepository for StubWorkspaceRepository {
        async fn materialize_user(
            &self,
            _principal: &AuthenticatedPrincipal,
        ) -> Result<UserRecord, WorkspaceStoreError> {
            unimplemented!("not used in session_service unit tests")
        }

        async fn bootstrap_workspace(
            &self,
            _owner_user_id: &str,
        ) -> Result<WorkspaceRecord, WorkspaceStoreError> {
            unimplemented!("not used in session_service unit tests")
        }

        async fn list_workspaces(
            &self,
            _owner_user_id: &str,
        ) -> Result<Vec<WorkspaceRecord>, WorkspaceStoreError> {
            unimplemented!("not used in session_service unit tests")
        }

        async fn load_workspace(
            &self,
            _owner_user_id: &str,
            _workspace_id: &str,
        ) -> Result<Option<WorkspaceRecord>, WorkspaceStoreError> {
            unimplemented!("not used in session_service unit tests")
        }

        async fn create_workspace(
            &self,
            _owner_user_id: &str,
            _workspace: &NewWorkspace,
        ) -> Result<WorkspaceRecord, WorkspaceStoreError> {
            unimplemented!("not used in session_service unit tests")
        }

        async fn update_workspace(
            &self,
            _owner_user_id: &str,
            _workspace_id: &str,
            _update: &WorkspaceUpdatePatch,
        ) -> Result<WorkspaceRecord, WorkspaceStoreError> {
            unimplemented!("not used in session_service unit tests")
        }

        async fn delete_workspace(
            &self,
            _owner_user_id: &str,
            _workspace_id: &str,
        ) -> Result<(), WorkspaceStoreError> {
            unimplemented!("not used in session_service unit tests")
        }

        async fn list_workspace_sessions(
            &self,
            _owner_user_id: &str,
            _workspace_id: &str,
        ) -> Result<Vec<crate::contract_sessions::SessionListItem>, WorkspaceStoreError> {
            unimplemented!("not used in session_service unit tests")
        }

        async fn save_session_metadata(
            &self,
            _record: &SessionMetadataRecord,
        ) -> Result<(), WorkspaceStoreError> {
            match &self.save_error {
                Some(error) => Err(error.clone()),
                None => Ok(()),
            }
        }

        async fn persist_session_snapshot(
            &self,
            _owner_user_id: &str,
            _snapshot: &SessionSnapshot,
            _touch_activity: bool,
            _status_override: Option<&str>,
        ) -> Result<(), WorkspaceStoreError> {
            Ok(())
        }

        async fn load_session_metadata(
            &self,
            _owner_user_id: &str,
            _session_id: &str,
        ) -> Result<Option<SessionMetadataRecord>, WorkspaceStoreError> {
            match &self.load_error {
                Some(error) => Err(error.clone()),
                None => Ok(self.metadata.clone()),
            }
        }

        async fn load_session_snapshot(
            &self,
            _owner_user_id: &str,
            _session_id: &str,
        ) -> Result<Option<DurableSessionSnapshotRecord>, WorkspaceStoreError> {
            unimplemented!("not used in session_service unit tests")
        }

        async fn auth_status(
            &self,
            _browser_session_id: Option<&str>,
        ) -> Result<(bool, Option<UserRecord>), WorkspaceStoreError> {
            unimplemented!("not used in session_service unit tests")
        }

        async fn authenticate_browser_session(
            &self,
            _browser_session_id: &str,
        ) -> Result<Option<UserRecord>, WorkspaceStoreError> {
            unimplemented!("not used in session_service unit tests")
        }

        async fn bootstrap_local_account(
            &self,
            _browser_session_id: &str,
            _username: &str,
            _password: &str,
        ) -> Result<LocalAccount, WorkspaceStoreError> {
            unimplemented!("not used in session_service unit tests")
        }

        async fn sign_in_local_account(
            &self,
            _browser_session_id: &str,
            _username: &str,
            _password: &str,
        ) -> Result<LocalAccount, WorkspaceStoreError> {
            unimplemented!("not used in session_service unit tests")
        }

        async fn sign_out_browser_session(
            &self,
            _browser_session_id: &str,
        ) -> Result<(), WorkspaceStoreError> {
            unimplemented!("not used in session_service unit tests")
        }

        async fn list_local_accounts(&self) -> Result<Vec<LocalAccount>, WorkspaceStoreError> {
            unimplemented!("not used in session_service unit tests")
        }

        async fn create_local_account(
            &self,
            _username: &str,
            _password: &str,
            _is_admin: bool,
        ) -> Result<LocalAccount, WorkspaceStoreError> {
            unimplemented!("not used in session_service unit tests")
        }

        async fn update_local_account(
            &self,
            _target_user_id: &str,
            _current_user_id: &str,
            _password: Option<&str>,
            _is_admin: Option<bool>,
        ) -> Result<LocalAccount, WorkspaceStoreError> {
            unimplemented!("not used in session_service unit tests")
        }

        async fn delete_local_account(
            &self,
            _target_user_id: &str,
            _current_user_id: &str,
        ) -> Result<Vec<String>, WorkspaceStoreError> {
            unimplemented!("not used in session_service unit tests")
        }
    }

    #[derive(Debug)]
    struct InvalidCheckoutManager;

    #[async_trait]
    impl WorkspaceCheckoutManager for InvalidCheckoutManager {
        async fn prepare_checkout(
            &self,
            _workspace: &WorkspaceRecord,
            _session_id: &str,
            _checkout_ref_override: Option<&str>,
        ) -> Result<PreparedWorkspaceCheckout, crate::workspace_checkout::WorkspaceCheckoutError>
        {
            unimplemented!("not used in session_service unit tests")
        }

        fn resolve_checkout_path(&self, _checkout_relpath: &str) -> Option<PathBuf> {
            None
        }
    }

    fn sample_user() -> UserRecord {
        let now = Utc::now();
        UserRecord {
            user_id: "u_test".to_string(),
            principal_kind: "bearer".to_string(),
            principal_subject: "alice".to_string(),
            username: Some("alice".to_string()),
            password_hash: None,
            is_admin: true,
            created_at: now,
            last_seen_at: now,
            deleted_at: None,
        }
    }

    fn sample_workspace() -> WorkspaceRecord {
        let now = Utc::now();
        WorkspaceRecord {
            workspace_id: "w_test".to_string(),
            owner_user_id: "u_test".to_string(),
            name: "Workspace".to_string(),
            upstream_url: None,
            default_ref: None,
            credential_reference_id: None,
            bootstrap_kind: None,
            status: "active".to_string(),
            created_at: now,
            updated_at: now,
            deleted_at: None,
        }
    }

    fn sample_metadata(checkout_relpath: Option<&str>) -> SessionMetadataRecord {
        let now = Utc::now();
        SessionMetadataRecord {
            session_id: "s_test".to_string(),
            workspace_id: "w_test".to_string(),
            owner_user_id: "u_test".to_string(),
            title: "Session".to_string(),
            status: "active".to_string(),
            checkout_relpath: checkout_relpath.map(str::to_string),
            checkout_ref: None,
            checkout_commit_sha: None,
            failure_reason: None,
            detach_deadline_at: None,
            restartable_deadline_at: None,
            created_at: now,
            last_activity_at: now,
            closed_at: None,
            deleted_at: None,
        }
    }

    fn sample_snapshot(session_id: &str) -> SessionSnapshot {
        SessionSnapshot {
            id: session_id.to_string(),
            workspace_id: "w_test".to_string(),
            title: "Session".to_string(),
            status: SessionStatus::Active,
            latest_sequence: 0,
            messages: Vec::new(),
            pending_permissions: Vec::new(),
        }
    }

    #[tokio::test]
    async fn provisioning_persistence_failures_roll_back_live_sessions() {
        let store = Arc::new(crate::sessions::SessionStore::new(4));
        let session = store
            .create_session("alice", "w_test")
            .await
            .expect("session creation should succeed");
        let state = AppState::with_workspace_repository(
            store.clone(),
            Arc::new(StubWorkspaceRepository {
                metadata: None,
                load_error: None,
                save_error: Some(WorkspaceStoreError::Database(
                    "metadata unavailable".to_string(),
                )),
            }),
            Arc::new(NoopReplyProvider),
        );
        let owner = super::super::OwnerContext {
            principal: AuthenticatedPrincipal {
                id: "alice".to_string(),
                kind: AuthenticatedPrincipalKind::Bearer,
                subject: "alice".to_string(),
            },
            user: sample_user(),
        };

        let error =
            persist_provisioning_session_lifecycle(&state, &owner, &sample_workspace(), &session)
                .await
                .expect_err("metadata failures should abort provisioning");

        assert!(matches!(error, AppError::Internal(message) if message == "internal server error"));
        assert_eq!(
            store
                .session_snapshot("alice", &session.id)
                .await
                .expect_err("failed provisioning should discard the session"),
            SessionStoreError::NotFound
        );
    }

    #[tokio::test]
    async fn failed_session_persistence_warnings_do_not_propagate() {
        let state = AppState::with_workspace_repository(
            Arc::new(crate::sessions::SessionStore::new(4)),
            Arc::new(StubWorkspaceRepository {
                metadata: None,
                load_error: None,
                save_error: Some(WorkspaceStoreError::Database(
                    "metadata unavailable".to_string(),
                )),
            }),
            Arc::new(NoopReplyProvider),
        );

        persist_failed_session_lifecycle(
            &state,
            &sample_user(),
            &sample_workspace(),
            &sample_snapshot("s_failed"),
            None,
            "checkout failed",
        )
        .await;
    }

    #[tokio::test]
    async fn checkout_cleanup_path_loading_handles_invalid_and_unreadable_metadata() {
        let invalid_path_state = AppState::with_workspace_repository_and_checkout_manager(
            Arc::new(crate::sessions::SessionStore::new(4)),
            Arc::new(StubWorkspaceRepository {
                metadata: Some(sample_metadata(Some("../escape"))),
                load_error: None,
                save_error: None,
            }),
            Arc::new(NoopReplyProvider),
            Arc::new(InvalidCheckoutManager),
        );
        let user = sample_user();

        assert_eq!(
            load_checkout_cleanup_path_best_effort(&invalid_path_state, &user, "s_test", "delete")
                .await,
            None
        );

        let load_error_state = AppState::with_workspace_repository(
            Arc::new(crate::sessions::SessionStore::new(4)),
            Arc::new(StubWorkspaceRepository {
                metadata: None,
                load_error: Some(WorkspaceStoreError::Database(
                    "metadata unavailable".to_string(),
                )),
                save_error: None,
            }),
            Arc::new(NoopReplyProvider),
        );

        assert_eq!(
            load_checkout_cleanup_path_best_effort(&load_error_state, &user, "s_test", "delete")
                .await,
            None
        );
    }

    #[test]
    fn cleanup_checkout_path_best_effort_ignores_missing_paths_and_files() {
        cleanup_checkout_path_best_effort(Path::new(
            "/workspace/.tmp/nonexistent-session-checkout",
        ));

        let file_path = std::env::current_dir()
            .expect("tests should start in a readable directory")
            .join(".tmp")
            .join(format!(
                "acp-session-cleanup-file-{}",
                uuid::Uuid::new_v4().simple()
            ));
        std::fs::create_dir_all(file_path.parent().expect("file path should have a parent"))
            .expect("parent dir should be creatable");
        std::fs::write(&file_path, "not a directory").expect("file path should be writable");

        cleanup_checkout_path_best_effort(&file_path);
        assert!(
            file_path.exists(),
            "file cleanups should fail without panicking"
        );
    }

    #[test]
    fn checkout_errors_map_to_public_http_errors() {
        assert!(matches!(
            map_checkout_error(crate::workspace_checkout::WorkspaceCheckoutError::Validation(
                "bad ref".to_string()
            )),
            AppError::BadRequest(message) if message == "bad ref"
        ));
        assert!(matches!(
            map_checkout_error(crate::workspace_checkout::WorkspaceCheckoutError::Io(
                "disk failed".to_string()
            )),
            AppError::Internal(message) if message == "checkout preparation failed"
        ));
        assert!(matches!(
            map_checkout_error(crate::workspace_checkout::WorkspaceCheckoutError::Git(
                "git failed".to_string()
            )),
            AppError::Internal(message) if message == "checkout preparation failed"
        ));
    }
}
