use acp_contracts::LocalAccount;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HomeRouteTarget {
    Register,
    SignIn,
    PrepareSession,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccountsRouteAccess {
    Admin(LocalAccount),
    RegisterRequired,
    SignInRequired,
    Forbidden,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccountConstraintReason {
    CurrentUser,
    LastAdmin,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountCapabilities {
    pub constraint: Option<AccountConstraintReason>,
}

impl AccountCapabilities {
    pub fn can_modify(&self) -> bool {
        self.constraint.is_none()
    }
}
