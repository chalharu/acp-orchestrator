use axum::{
    Json,
    extract::{Extension, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};

use crate::auth::AuthenticatedPrincipal;
use crate::contract_accounts::{
    AuthStatusResponse, BootstrapRegistrationRequest, BootstrapRegistrationResponse, SignInRequest,
    SignInResponse,
};

use super::super::{
    AppError, AppState,
    account_service::{
        forget_live_sessions_for_owners, require_browser_session, user_record_to_local_account,
    },
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
    require_browser_session(
        &principal,
        "bootstrap registration requires a browser session",
    )?;
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
    require_browser_session(&principal, "password sign-in requires a browser session")?;
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
    require_browser_session(&principal, "sign-out requires a browser session")?;
    state
        .workspace_repository
        .sign_out_browser_session(&principal.id)
        .await?;
    forget_live_sessions_for_owners(&state, std::slice::from_ref(&principal.id)).await;
    Ok((StatusCode::NO_CONTENT, sign_out_response_headers()).into_response())
}
