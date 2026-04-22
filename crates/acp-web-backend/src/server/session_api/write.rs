use axum::{
    Json,
    extract::{Extension, Path, State},
};

use crate::auth::AuthenticatedPrincipal;
use crate::contract_messages::{PromptRequest, PromptResponse};
use crate::contract_permissions::{ResolvePermissionRequest, ResolvePermissionResponse};
use crate::contract_sessions::{
    CancelTurnResponse, CloseSessionResponse, CreateSessionResponse, DeleteSessionResponse,
    RenameSessionRequest, RenameSessionResponse,
};

use super::super::{
    AppError, AppState,
    session_service::{
        close_live_session, create_session_snapshot, delete_live_session, rename_session_title,
        submit_prompt,
    },
};

pub(in crate::server) async fn create_session(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
) -> Result<(axum::http::StatusCode, Json<CreateSessionResponse>), AppError> {
    let session = create_session_snapshot(&state, principal).await?;

    Ok((
        axum::http::StatusCode::CREATED,
        Json(CreateSessionResponse { session }),
    ))
}

pub(in crate::server) async fn rename_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
    Json(request): Json<RenameSessionRequest>,
) -> Result<Json<RenameSessionResponse>, AppError> {
    let session = rename_session_title(&state, principal, &session_id, request.title).await?;

    Ok(Json(RenameSessionResponse { session }))
}

pub(in crate::server) async fn delete_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
) -> Result<Json<DeleteSessionResponse>, AppError> {
    delete_live_session(&state, principal, &session_id).await?;

    Ok(Json(DeleteSessionResponse { deleted: true }))
}

pub(in crate::server) async fn post_message(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
    Json(request): Json<PromptRequest>,
) -> Result<Json<PromptResponse>, AppError> {
    submit_prompt(&state, principal, &session_id, request.text).await?;

    Ok(Json(PromptResponse { accepted: true }))
}

pub(in crate::server) async fn close_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
) -> Result<Json<CloseSessionResponse>, AppError> {
    let session = close_live_session(&state, principal, &session_id).await?;

    Ok(Json(CloseSessionResponse { session }))
}

pub(in crate::server) async fn cancel_turn(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
) -> Result<Json<CancelTurnResponse>, AppError> {
    let owner = state.owner_context(principal).await?;
    let cancelled = state
        .store
        .cancel_active_turn(&owner.principal.id, &session_id)
        .await?;

    Ok(Json(CancelTurnResponse { cancelled }))
}

pub(in crate::server) async fn resolve_permission(
    State(state): State<AppState>,
    Path((session_id, request_id)): Path<(String, String)>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
    Json(request): Json<ResolvePermissionRequest>,
) -> Result<Json<ResolvePermissionResponse>, AppError> {
    let owner = state.owner_context(principal).await?;
    let resolution = state
        .store
        .resolve_permission(
            &owner.principal.id,
            &session_id,
            &request_id,
            request.decision,
        )
        .await?;

    Ok(Json(resolution))
}
