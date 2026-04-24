use axum::{
    Json,
    extract::{Extension, Path, State},
};

use crate::{
    auth::AuthenticatedPrincipal,
    contract_sessions::SessionListResponse,
    contract_workspaces::{WorkspaceListResponse, WorkspaceResponse},
};

use super::super::{
    AppError, AppState,
    workspace_service::{workspace_detail, workspace_summary},
};

pub(in crate::server) async fn list_workspaces(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
) -> Result<Json<WorkspaceListResponse>, AppError> {
    let owner = state.owner_context(principal).await?;
    let workspaces = state
        .workspace_repository
        .list_workspaces(&owner.user.user_id)
        .await?
        .into_iter()
        .map(workspace_summary)
        .collect();

    Ok(Json(WorkspaceListResponse { workspaces }))
}

pub(in crate::server) async fn get_workspace(
    State(state): State<AppState>,
    Path(workspace_id): Path<String>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
) -> Result<Json<WorkspaceResponse>, AppError> {
    let owner = state.owner_context(principal).await?;
    let workspace = state
        .workspace_repository
        .load_workspace(&owner.user.user_id, &workspace_id)
        .await?
        .ok_or_else(|| AppError::NotFound("workspace not found".to_string()))?;

    Ok(Json(WorkspaceResponse {
        workspace: workspace_detail(workspace),
    }))
}

pub(in crate::server) async fn list_workspace_sessions(
    State(state): State<AppState>,
    Path(workspace_id): Path<String>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
) -> Result<Json<SessionListResponse>, AppError> {
    let owner = state.owner_context(principal).await?;
    state
        .workspace_repository
        .load_workspace(&owner.user.user_id, &workspace_id)
        .await?
        .ok_or_else(|| AppError::NotFound("workspace not found".to_string()))?;
    let sessions = state
        .workspace_repository
        .list_workspace_sessions(&owner.user.user_id, &workspace_id)
        .await?;

    Ok(Json(SessionListResponse { sessions }))
}
