use axum::{
    Json,
    extract::{Extension, Path, Query, State},
};

use crate::contract_sessions::{SessionHistoryResponse, SessionListResponse, SessionResponse};
use crate::contract_slash::SlashCompletionsResponse;
use crate::{auth::AuthenticatedPrincipal, completions::resolve_slash_completions};

use super::super::{AppError, AppState, assets::SlashCompletionsQuery};

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
    let session = state
        .store
        .session_snapshot(&owner.principal.id, &session_id)
        .await?;

    Ok(Json(SessionResponse { session }))
}

pub(in crate::server) async fn get_session_history(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
) -> Result<Json<SessionHistoryResponse>, AppError> {
    let owner = state.owner_context(principal).await?;
    let messages = state
        .store
        .session_history(&owner.principal.id, &session_id)
        .await?;

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
