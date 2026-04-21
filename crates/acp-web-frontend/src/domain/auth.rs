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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountCapabilities {
    pub can_delete: bool,
    pub can_toggle_admin: bool,
}
