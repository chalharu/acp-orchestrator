use axum::{
    Json,
    extract::{Extension, Path, Query, State},
};

use crate::contract_sessions::{
    SessionHistoryResponse, SessionListResponse, SessionResponse, SessionSnapshot,
};
use crate::contract_slash::SlashCompletionsResponse;
use crate::{
    auth::AuthenticatedPrincipal, completions::resolve_slash_completions,
    sessions::SessionStoreError,
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
    let Some(durable) = state
        .workspace_repository
        .load_session_snapshot(&owner.user.user_id, session_id)
        .await?
    else {
        return Err(AppError::NotFound("session not found".to_string()));
    };
    let crate::workspace_records::DurableSessionSnapshotRecord {
        session,
        last_activity_at,
    } = durable;

    state
        .store
        .restore_session(&owner.principal.id, session, last_activity_at)
        .await
        .map_err(AppError::from)
}
