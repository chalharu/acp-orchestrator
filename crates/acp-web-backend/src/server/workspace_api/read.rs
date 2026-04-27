use axum::{
    Json,
    extract::{Extension, Path, State},
};

use crate::{
    auth::AuthenticatedPrincipal,
    contract_sessions::SessionListResponse,
    contract_workspaces::{WorkspaceBranchListResponse, WorkspaceListResponse, WorkspaceResponse},
    workspace_checkout::WorkspaceCheckoutError,
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

pub(in crate::server) async fn list_workspace_branches(
    State(state): State<AppState>,
    Path(workspace_id): Path<String>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
) -> Result<Json<WorkspaceBranchListResponse>, AppError> {
    let owner = state.owner_context(principal).await?;
    let workspace = state
        .workspace_repository
        .load_workspace(&owner.user.user_id, &workspace_id)
        .await?
        .ok_or_else(|| AppError::NotFound("workspace not found".to_string()))?;
    let branches = state
        .checkout_manager
        .list_branches(&workspace)
        .await
        .map_err(map_workspace_branch_error)?;

    Ok(Json(WorkspaceBranchListResponse { branches }))
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

fn map_workspace_branch_error(error: WorkspaceCheckoutError) -> AppError {
    match error {
        WorkspaceCheckoutError::Validation(message) => AppError::BadRequest(message),
        WorkspaceCheckoutError::Io(message) | WorkspaceCheckoutError::Git(message) => {
            tracing::error!(error = %message, "workspace branch lookup failed");
            AppError::Internal("workspace branch lookup failed".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_branch_errors_map_validation_to_bad_request() {
        let error = map_workspace_branch_error(WorkspaceCheckoutError::Validation(
            "invalid branch".to_string(),
        ));

        assert!(matches!(error, AppError::BadRequest(message) if message == "invalid branch"));
    }

    #[test]
    fn workspace_branch_errors_map_io_and_git_to_internal() {
        for error in [
            WorkspaceCheckoutError::Io("io failure".to_string()),
            WorkspaceCheckoutError::Git("git failure".to_string()),
        ] {
            let mapped = map_workspace_branch_error(error);
            assert!(
                matches!(mapped, AppError::Internal(message) if message == "workspace branch lookup failed")
            );
        }
    }
}
