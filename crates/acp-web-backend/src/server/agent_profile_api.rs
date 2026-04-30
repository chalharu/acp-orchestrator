use axum::{
    Json,
    extract::{Extension, Path, State},
    http::StatusCode,
};

use crate::{
    auth::AuthenticatedPrincipal,
    contract_sessions::{
        AgentProfileListResponse, AgentProfileResponse, UpsertAgentProfileRequest,
    },
};

use super::{AppError, AppState, account_service::require_admin};

pub(in crate::server) async fn list_agent_profiles(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
) -> Result<Json<AgentProfileListResponse>, AppError> {
    let owner = state.owner_context(principal).await?;
    let profiles = state
        .agent_profile_store
        .list_profiles()
        .map_err(map_profile_store_error)?;
    let response = AgentProfileListResponse {
        profiles,
        can_manage: owner.user.is_admin,
    };
    Ok(Json(response))
}

pub(in crate::server) async fn create_agent_profile(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
    Json(request): Json<UpsertAgentProfileRequest>,
) -> Result<(StatusCode, Json<AgentProfileResponse>), AppError> {
    let owner = state.owner_context(principal).await?;
    require_admin(&owner)?;
    let profile = state
        .agent_profile_store
        .create_profile(request)
        .map_err(map_profile_store_error)?;
    Ok((StatusCode::CREATED, Json(AgentProfileResponse { profile })))
}

pub(in crate::server) async fn upsert_agent_profile(
    State(state): State<AppState>,
    Path(profile_id): Path<String>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
    Json(request): Json<UpsertAgentProfileRequest>,
) -> Result<Json<AgentProfileResponse>, AppError> {
    let owner = state.owner_context(principal).await?;
    require_admin(&owner)?;
    let profile = state
        .agent_profile_store
        .upsert_profile(&profile_id, request)
        .map_err(map_profile_store_error)?;
    Ok(Json(AgentProfileResponse { profile }))
}

fn map_profile_store_error(error: crate::agent_profiles::AgentProfileStoreError) -> AppError {
    match error {
        crate::agent_profiles::AgentProfileStoreError::NotFound => {
            AppError::NotFound(error.message().to_string())
        }
        crate::agent_profiles::AgentProfileStoreError::Validation(message) => {
            AppError::BadRequest(message)
        }
        crate::agent_profiles::AgentProfileStoreError::Io(message)
        | crate::agent_profiles::AgentProfileStoreError::Json(message) => {
            AppError::Internal(message)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_profiles::AgentProfileStoreError;

    #[test]
    fn profile_store_errors_map_to_http_errors() {
        assert!(matches!(
            map_profile_store_error(AgentProfileStoreError::NotFound),
            AppError::NotFound(message) if message == "agent profile not found"
        ));
        assert!(matches!(
            map_profile_store_error(AgentProfileStoreError::Validation("bad".to_string())),
            AppError::BadRequest(message) if message == "bad"
        ));
        assert!(matches!(
            map_profile_store_error(AgentProfileStoreError::Io("io".to_string())),
            AppError::Internal(message) if message == "io"
        ));
        assert!(matches!(
            map_profile_store_error(AgentProfileStoreError::Json("json".to_string())),
            AppError::Internal(message) if message == "json"
        ));
    }
}
