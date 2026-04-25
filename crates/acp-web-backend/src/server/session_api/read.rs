use axum::{
    Json,
    extract::{Extension, Path, Query, State},
};

use crate::contract_sessions::{
    SessionHistoryResponse, SessionListResponse, SessionResponse, SessionSnapshot,
};
use crate::contract_slash::SlashCompletionsResponse;
use crate::{
    auth::AuthenticatedPrincipal,
    completions::resolve_slash_completions,
    sessions::SessionStoreError,
    workspace_records::{DurableSessionSnapshotRecord, SessionMetadataRecord},
};

use super::super::{AppError, AppState, OwnerContext, assets::SlashCompletionsQuery};

pub(in crate::server) async fn list_sessions(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
) -> Result<Json<SessionListResponse>, AppError> {
    let owner = state.owner_context(principal).await?;
    let sessions = state.store.list_owned_sessions(&owner.principal.id).await;

    Ok(Json(SessionListResponse { sessions }))
}

pub(in crate::server) async fn get_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
) -> Result<Json<SessionResponse>, AppError> {
    let owner = state.owner_context(principal).await?;
    let session = load_or_restore_session(&state, &owner, &session_id).await?;

    Ok(Json(SessionResponse { session }))
}

pub(in crate::server) async fn get_session_history(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
) -> Result<Json<SessionHistoryResponse>, AppError> {
    let owner = state.owner_context(principal).await?;
    let messages = load_or_restore_session(&state, &owner, &session_id)
        .await?
        .messages;

    Ok(Json(SessionHistoryResponse {
        session_id,
        messages,
    }))
}

pub(in crate::server) async fn get_slash_completions(
    State(state): State<AppState>,
    Query(query): Query<SlashCompletionsQuery>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
) -> Result<Json<SlashCompletionsResponse>, AppError> {
    let owner = state.owner_context(principal).await?;
    let response = resolve_slash_completions(
        &state.store,
        &owner.principal.id,
        &query.session_id,
        &query.prefix,
    )
    .await?;

    Ok(Json(response))
}

async fn load_or_restore_session(
    state: &AppState,
    owner: &OwnerContext,
    session_id: &str,
) -> Result<SessionSnapshot, AppError> {
    match state
        .store
        .session_snapshot(&owner.principal.id, session_id)
        .await
    {
        Ok(session) => Ok(session),
        Err(SessionStoreError::NotFound) => restore_durable_session(state, owner, session_id).await,
        Err(error) => Err(error.into()),
    }
}

async fn restore_durable_session(
    state: &AppState,
    owner: &OwnerContext,
    session_id: &str,
) -> Result<SessionSnapshot, AppError> {
    let (metadata, durable) = load_restorable_session(state, owner, session_id).await?;
    let restored = restore_durable_snapshot(state, owner, durable).await?;
    bind_restored_session_to_checkout(state, &owner.principal.id, &restored, metadata.as_ref())
        .await?;
    Ok(restored)
}

async fn load_restorable_session(
    state: &AppState,
    owner: &OwnerContext,
    session_id: &str,
) -> Result<(Option<SessionMetadataRecord>, DurableSessionSnapshotRecord), AppError> {
    let metadata = state
        .workspace_repository
        .load_session_metadata(&owner.user.user_id, session_id)
        .await?;
    let durable = state
        .workspace_repository
        .load_session_snapshot(&owner.user.user_id, session_id)
        .await?
        .ok_or_else(|| AppError::NotFound("session not found".to_string()))?;
    Ok((metadata, durable))
}

async fn restore_durable_snapshot(
    state: &AppState,
    owner: &OwnerContext,
    durable: DurableSessionSnapshotRecord,
) -> Result<SessionSnapshot, AppError> {
    let DurableSessionSnapshotRecord {
        session,
        last_activity_at,
    } = durable;
    state
        .store
        .restore_session(&owner.principal.id, session, last_activity_at)
        .await
        .map_err(AppError::from)
}

async fn bind_restored_session_to_checkout(
    state: &AppState,
    owner_id: &str,
    restored: &SessionSnapshot,
    metadata: Option<&SessionMetadataRecord>,
) -> Result<(), AppError> {
    let Some(checkout_relpath) = metadata.and_then(|record| record.checkout_relpath.as_deref())
    else {
        return Ok(());
    };
    let checkout_path =
        resolve_restored_checkout_path(state, owner_id, restored, checkout_relpath).await?;
    match state
        .reply_provider
        .bind_session(&restored.id, checkout_path)
        .await
    {
        Ok(()) => Ok(()),
        Err(error) => {
            rollback_restored_session(state, owner_id, &restored.id, &error).await?;
            Err(AppError::Internal(error))
        }
    }
}

async fn resolve_restored_checkout_path(
    state: &AppState,
    owner_id: &str,
    restored: &SessionSnapshot,
    checkout_relpath: &str,
) -> Result<std::path::PathBuf, AppError> {
    match state
        .checkout_manager
        .resolve_checkout_path(checkout_relpath)
    {
        Some(path) => Ok(path),
        None => {
            let error = AppError::Internal("persisted checkout path is invalid".to_string());
            rollback_restored_session(state, owner_id, &restored.id, error.message()).await?;
            Err(error)
        }
    }
}

async fn rollback_restored_session(
    state: &AppState,
    owner_id: &str,
    session_id: &str,
    error_message: &str,
) -> Result<(), AppError> {
    state.reply_provider.forget_session(session_id);
    state
        .store
        .discard_session(owner_id, session_id)
        .await
        .map_err(|rollback_error| {
            AppError::Internal(format!(
                "{error_message}; restored session rollback failed: {}",
                rollback_error.message()
            ))
        })
}
