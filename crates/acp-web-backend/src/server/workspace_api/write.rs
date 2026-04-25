use axum::{
    Json,
    extract::{Extension, Path, State},
};

use crate::{
    auth::AuthenticatedPrincipal,
    contract_sessions::CreateSessionResponse,
    contract_workspaces::{
        CreateWorkspaceRequest, CreateWorkspaceResponse, DeleteWorkspaceResponse,
        UpdateWorkspaceRequest, UpdateWorkspaceResponse,
    },
    workspace_repository::{NewWorkspace, WorkspaceUpdatePatch},
};

use super::super::{
    AppError, AppState, session_service::create_session_snapshot_for_workspace,
    workspace_service::workspace_detail,
};

pub(in crate::server) async fn create_workspace(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
    Json(request): Json<CreateWorkspaceRequest>,
) -> Result<(axum::http::StatusCode, Json<CreateWorkspaceResponse>), AppError> {
    let owner = state.owner_context(principal).await?;
    let workspace_request = NewWorkspace {
        name: request.name,
        upstream_url: request.upstream_url,
        default_ref: request.default_ref,
        credential_reference_id: request.credential_reference_id,
    };
    let workspace = state
        .workspace_repository
        .create_workspace(&owner.user.user_id, &workspace_request)
        .await?;

    Ok((
        axum::http::StatusCode::CREATED,
        Json(CreateWorkspaceResponse {
            workspace: workspace_detail(workspace),
        }),
    ))
}

pub(in crate::server) async fn update_workspace(
    State(state): State<AppState>,
    Path(workspace_id): Path<String>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
    Json(request): Json<UpdateWorkspaceRequest>,
) -> Result<Json<UpdateWorkspaceResponse>, AppError> {
    let owner = state.owner_context(principal).await?;
    let workspace_update = WorkspaceUpdatePatch {
        name: request.name,
        default_ref: request.default_ref,
    };
    let workspace = state
        .workspace_repository
        .update_workspace(&owner.user.user_id, &workspace_id, &workspace_update)
        .await?;

    Ok(Json(UpdateWorkspaceResponse {
        workspace: workspace_detail(workspace),
    }))
}

pub(in crate::server) async fn delete_workspace(
    State(state): State<AppState>,
    Path(workspace_id): Path<String>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
) -> Result<Json<DeleteWorkspaceResponse>, AppError> {
    let owner = state.owner_context(principal).await?;
    state
        .workspace_repository
        .delete_workspace(&owner.user.user_id, &workspace_id)
        .await?;

    Ok(Json(DeleteWorkspaceResponse { deleted: true }))
}

pub(in crate::server) async fn create_workspace_session(
    State(state): State<AppState>,
    Path(workspace_id): Path<String>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
) -> Result<(axum::http::StatusCode, Json<CreateSessionResponse>), AppError> {
    let session = create_session_snapshot_for_workspace(&state, principal, &workspace_id).await?;

    Ok((
        axum::http::StatusCode::CREATED,
        Json(CreateSessionResponse { session }),
    ))
}
