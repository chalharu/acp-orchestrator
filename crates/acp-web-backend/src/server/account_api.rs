use axum::{
    Json,
    extract::{Extension, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};

use acp_contracts::{
    AccountListResponse, AuthStatusResponse, BootstrapRegistrationRequest,
    BootstrapRegistrationResponse, CreateAccountRequest, CreateAccountResponse,
    DeleteAccountResponse, LocalAccount, SignInRequest, SignInResponse, UpdateAccountRequest,
    UpdateAccountResponse,
};

use crate::{auth::AuthenticatedPrincipal, workspace_records::UserRecord};

use super::{
    AppError, AppState, OwnerContext,
    assets::{current_browser_session_id, sign_out_response_headers},
};

pub(super) async fn auth_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<AuthStatusResponse>, AppError> {
    let (bootstrap_required, account) = state
        .workspace_repository
        .auth_status(current_browser_session_id(&headers).as_deref())
        .await?;
    Ok(Json(AuthStatusResponse {
        bootstrap_required,
        account: account
            .as_ref()
            .map(user_record_to_local_account)
            .transpose()?,
    }))
}

pub(super) async fn bootstrap_register(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
    Json(request): Json<BootstrapRegistrationRequest>,
) -> Result<(StatusCode, Json<BootstrapRegistrationResponse>), AppError> {
    if !matches!(
        principal.kind,
        crate::auth::AuthenticatedPrincipalKind::BrowserSession
    ) {
        return Err(AppError::Forbidden(
            "bootstrap registration requires a browser session".to_string(),
        ));
    }
    let account = state
        .workspace_repository
        .bootstrap_local_account(&principal.id, &request.username, &request.password)
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(BootstrapRegistrationResponse { account }),
    ))
}

pub(super) async fn sign_in(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
    Json(request): Json<SignInRequest>,
) -> Result<Json<SignInResponse>, AppError> {
    if !matches!(
        principal.kind,
        crate::auth::AuthenticatedPrincipalKind::BrowserSession
    ) {
        return Err(AppError::Forbidden(
            "password sign-in requires a browser session".to_string(),
        ));
    }
    let account = state
        .workspace_repository
        .sign_in_local_account(&principal.id, &request.username, &request.password)
        .await?;
    forget_live_sessions_for_owners(&state, std::slice::from_ref(&principal.id)).await;
    Ok(Json(SignInResponse { account }))
}

pub(super) async fn sign_out(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
) -> Result<Response, AppError> {
    if !matches!(
        principal.kind,
        crate::auth::AuthenticatedPrincipalKind::BrowserSession
    ) {
        return Err(AppError::Forbidden(
            "sign-out requires a browser session".to_string(),
        ));
    }
    state
        .workspace_repository
        .sign_out_browser_session(&principal.id)
        .await?;
    forget_live_sessions_for_owners(&state, std::slice::from_ref(&principal.id)).await;
    Ok((StatusCode::NO_CONTENT, sign_out_response_headers()).into_response())
}

pub(super) async fn list_accounts(
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

pub(super) async fn create_account(
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

pub(super) async fn update_account(
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

pub(super) async fn delete_account(
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

fn require_admin(owner: &OwnerContext) -> Result<(), AppError> {
    if owner.user.is_admin {
        Ok(())
    } else {
        Err(AppError::Forbidden("admin access required".to_string()))
    }
}

fn user_record_to_local_account(user: &UserRecord) -> Result<LocalAccount, AppError> {
    Ok(LocalAccount {
        user_id: user.user_id.clone(),
        username: user
            .username
            .clone()
            .ok_or_else(|| AppError::Internal("local account missing username".to_string()))?,
        is_admin: user.is_admin,
        created_at: user.created_at,
    })
}

async fn forget_live_sessions_for_owners(state: &AppState, owner_ids: &[String]) {
    let invalidated_sessions = state.store.delete_sessions_for_owners(owner_ids).await;
    for session_id in invalidated_sessions {
        state.reply_provider.forget_session(&session_id);
    }
}
