use axum::{
    Json,
    extract::{Extension, State},
    http::StatusCode,
};

use crate::auth::AuthenticatedPrincipal;
use crate::contract_accounts::{
    AccountListResponse, CreateAccountRequest, CreateAccountResponse, DeleteAccountResponse,
    UpdateAccountRequest, UpdateAccountResponse,
};

use super::super::{
    AppError, AppState,
    account_service::{forget_live_sessions_for_owners, require_admin},
};

pub(in crate::server) async fn list_accounts(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
) -> Result<Json<AccountListResponse>, AppError> {
    let owner = state.owner_context(principal).await?;
    require_admin(&owner)?;
    let accounts = state.workspace_repository.list_local_accounts().await?;
    Ok(Json(AccountListResponse {
        current_user_id: owner.user.user_id,
        accounts,
    }))
}

pub(in crate::server) async fn create_account(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
    Json(request): Json<CreateAccountRequest>,
) -> Result<(StatusCode, Json<CreateAccountResponse>), AppError> {
    let owner = state.owner_context(principal).await?;
    require_admin(&owner)?;
    let account = state
        .workspace_repository
        .create_local_account(&request.username, &request.password, request.is_admin)
        .await?;
    Ok((StatusCode::CREATED, Json(CreateAccountResponse { account })))
}

pub(in crate::server) async fn update_account(
    State(state): State<AppState>,
    axum::extract::Path(user_id): axum::extract::Path<String>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
    Json(request): Json<UpdateAccountRequest>,
) -> Result<Json<UpdateAccountResponse>, AppError> {
    let owner = state.owner_context(principal).await?;
    require_admin(&owner)?;
    let account = state
        .workspace_repository
        .update_local_account(
            &user_id,
            &owner.user.user_id,
            request.password.as_deref(),
            request.is_admin,
        )
        .await?;
    Ok(Json(UpdateAccountResponse { account }))
}

pub(in crate::server) async fn delete_account(
    State(state): State<AppState>,
    axum::extract::Path(user_id): axum::extract::Path<String>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
) -> Result<Json<DeleteAccountResponse>, AppError> {
    let owner = state.owner_context(principal).await?;
    require_admin(&owner)?;
    let invalidated_browser_sessions = state
        .workspace_repository
        .delete_local_account(&user_id, &owner.user.user_id)
        .await?;
    forget_live_sessions_for_owners(&state, &invalidated_browser_sessions).await;
    Ok(Json(DeleteAccountResponse { deleted: true }))
}
