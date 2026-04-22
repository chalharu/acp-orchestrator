use crate::{
    auth::AuthenticatedPrincipal, contract_accounts::LocalAccount, workspace_records::UserRecord,
};

use super::{AppError, AppState, OwnerContext};

pub(super) fn require_browser_session(
    principal: &AuthenticatedPrincipal,
    message: &str,
) -> Result<(), AppError> {
    if matches!(
        principal.kind,
        crate::auth::AuthenticatedPrincipalKind::BrowserSession
    ) {
        Ok(())
    } else {
        Err(AppError::Forbidden(message.to_string()))
    }
}

pub(super) fn require_admin(owner: &OwnerContext) -> Result<(), AppError> {
    if owner.user.is_admin {
        Ok(())
    } else {
        Err(AppError::Forbidden("admin access required".to_string()))
    }
}

pub(super) fn user_record_to_local_account(user: &UserRecord) -> Result<LocalAccount, AppError> {
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

pub(super) async fn forget_live_sessions_for_owners(state: &AppState, owner_ids: &[String]) {
    let invalidated_sessions = state.store.delete_sessions_for_owners(owner_ids).await;
    for session_id in invalidated_sessions {
        state.reply_provider.forget_session(&session_id);
    }
}
