use std::sync::Arc;

use crate::{
    auth::{AuthenticatedPrincipal, AuthenticatedPrincipalKind},
    contract_sessions::SessionSnapshot,
    contract_workspaces::CreateWorkspaceRequest,
    mock_client::{ReplyProvider, ReplyResult},
    sessions::{PendingPrompt, SessionStoreError},
    workspace_records::UserRecord,
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
                .create_workspace(&owner.user.user_id, &legacy_session_workspace_request())
                .await?
                .workspace_id
        }
        _ => {
            return Err(AppError::Conflict(
                "workspace selection required".to_string(),
            ));
        }
    };

    create_session_snapshot_in_workspace(state, owner, &workspace_id).await
}

fn legacy_session_workspace_request() -> CreateWorkspaceRequest {
    CreateWorkspaceRequest {
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
) -> Result<SessionSnapshot, AppError> {
    let owner = state.owner_context(principal).await?;
    create_session_snapshot_in_workspace(state, owner, workspace_id).await
}

async fn create_session_snapshot_in_workspace(
    state: &AppState,
    owner: super::OwnerContext,
    workspace_id: &str,
) -> Result<SessionSnapshot, AppError> {
    state
        .workspace_repository
        .load_workspace(&owner.user.user_id, workspace_id)
        .await?
        .ok_or_else(|| AppError::NotFound("workspace not found".to_string()))?;
    let session = state
        .store
        .create_session(&owner.principal.id, workspace_id)
        .await?;
    let session_id = session.id.clone();
    let session = match seed_startup_hint(state, &owner.principal.id, session).await {
        Ok(session) => session,
        Err(error) => {
            if let Err(rollback_error) =
                rollback_failed_session(state, &owner.principal.id, &session_id).await
            {
                return Err(AppError::Internal(format!(
                    "{}; session rollback failed: {}",
                    error.message(),
                    rollback_error.message()
                )));
            }
            return Err(error);
        }
    };
    if let Err(error) = persist_session_metadata(state, &owner.user, &session, true, None).await {
        if let Err(rollback_error) =
            rollback_failed_session(state, &owner.principal.id, &session_id).await
        {
            return Err(AppError::Internal(format!(
                "{}; session rollback failed: {}",
                error.message(),
                rollback_error.message()
            )));
        }
        return Err(error);
    }

    Ok(session)
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
    dispatch_assistant_request(state.reply_provider.clone(), pending);

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
    match snapshot_result {
        Ok(snapshot) => {
            persist_session_metadata_best_effort(
                state,
                user,
                &snapshot,
                true,
                None,
                "submit_prompt",
            )
            .await;
        }
        Err(error) => {
            let error_message = error.message();
            tracing::warn!(
                session_id = %session_id,
                "failed to snapshot session metadata after prompt submission: {error_message}"
            );
        }
    }
}

async fn seed_startup_hint(
    state: &AppState,
    owner: &str,
    session: SessionSnapshot,
) -> Result<SessionSnapshot, AppError> {
    if !state.startup_hints {
        return Ok(session);
    }

    let Some(hint) = state
        .reply_provider
        .prime_session(&session.id)
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?
    else {
        return Ok(session);
    };

    state
        .store
        .append_assistant_message(owner, &session.id, hint)
        .await
        .map_err(AppError::from)
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

fn dispatch_assistant_request(reply_provider: Arc<dyn ReplyProvider>, pending: PendingPrompt) {
    tokio::spawn(async move {
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
    });
}
