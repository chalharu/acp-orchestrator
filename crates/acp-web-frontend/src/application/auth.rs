use acp_contracts::{AuthStatusResponse, LocalAccount};

use crate::domain::auth::{
    AccountCapabilities, AccountConstraintReason, AccountsRouteAccess, HomeRouteTarget,
};

pub fn home_route_target(status: &AuthStatusResponse) -> HomeRouteTarget {
    if status.account.is_some() {
        HomeRouteTarget::PrepareSession
    } else if status.bootstrap_required {
        HomeRouteTarget::Register
    } else {
        HomeRouteTarget::SignIn
    }
}

pub fn accounts_route_access(status: &AuthStatusResponse) -> AccountsRouteAccess {
    match &status.account {
        Some(account) if account.is_admin => AccountsRouteAccess::Admin(account.clone()),
        Some(_) => AccountsRouteAccess::Forbidden,
        None if status.bootstrap_required => AccountsRouteAccess::RegisterRequired,
        None => AccountsRouteAccess::SignInRequired,
    }
}

pub fn account_capabilities(
    current_user_id: &str,
    accounts: &[LocalAccount],
    account: &LocalAccount,
) -> AccountCapabilities {
    let admin_count = accounts
        .iter()
        .filter(|candidate| candidate.is_admin)
        .count();
    let is_current_user = account.user_id == current_user_id;
    let removing_last_admin = account.is_admin && admin_count <= 1;
    let constraint = if is_current_user {
        Some(AccountConstraintReason::CurrentUser)
    } else if removing_last_admin {
        Some(AccountConstraintReason::LastAdmin)
    } else {
        None
    };

    AccountCapabilities { constraint }
}

#[cfg(test)]
mod tests {
    use acp_contracts::{AuthStatusResponse, LocalAccount};
    use chrono::{TimeZone, Utc};

    use super::*;

    fn sample_account(user_id: &str, is_admin: bool) -> LocalAccount {
        LocalAccount {
            user_id: user_id.to_string(),
            username: user_id.to_string(),
            is_admin,
            created_at: Utc.with_ymd_and_hms(2026, 4, 17, 1, 0, 0).unwrap(),
        }
    }

    #[test]
    fn home_target_routes_bootstrap_to_register() {
        assert_eq!(
            home_route_target(&AuthStatusResponse {
                bootstrap_required: true,
                account: None,
            }),
            HomeRouteTarget::Register
        );
    }

    #[test]
    fn home_target_routes_signed_out_users_to_sign_in() {
        assert_eq!(
            home_route_target(&AuthStatusResponse {
                bootstrap_required: false,
                account: None,
            }),
            HomeRouteTarget::SignIn
        );
    }

    #[test]
    fn accounts_access_requires_admin() {
        assert_eq!(
            accounts_route_access(&AuthStatusResponse {
                bootstrap_required: false,
                account: Some(sample_account("user", false)),
            }),
            AccountsRouteAccess::Forbidden
        );
        assert_eq!(
            accounts_route_access(&AuthStatusResponse {
                bootstrap_required: false,
                account: None,
            }),
            AccountsRouteAccess::SignInRequired
        );
    }

    #[test]
    fn account_capabilities_block_self_delete_and_last_admin_loss() {
        let admin = sample_account("admin", true);
        let second_admin = sample_account("admin-2", true);
        let member = sample_account("member", false);
        assert_eq!(
            account_capabilities(&admin.user_id, &[admin.clone(), member.clone()], &admin),
            AccountCapabilities {
                constraint: Some(AccountConstraintReason::CurrentUser),
            }
        );
        assert_eq!(
            account_capabilities(
                &member.user_id.clone(),
                &[admin.clone(), second_admin, member],
                &admin,
            ),
            AccountCapabilities {
                constraint: None,
            }
        );
        assert_eq!(
            account_capabilities("other", std::slice::from_ref(&admin), &admin),
            AccountCapabilities {
                constraint: Some(AccountConstraintReason::LastAdmin),
            }
        );
    }
}
