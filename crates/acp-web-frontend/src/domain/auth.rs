use acp_contracts::AuthSessionResponse;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AuthSession {
    pub(crate) user_name: String,
    pub(crate) is_admin: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RegistrationRouteAccess {
    Register,
    SignIn,
    Unavailable,
}

pub(crate) fn auth_session_from_response(
    response: AuthSessionResponse,
) -> Result<Option<AuthSession>, String> {
    if !response.authenticated {
        return Ok(None);
    }

    let user_name = response
        .user_name
        .map(|user_name| user_name.trim().to_string())
        .filter(|user_name| !user_name.is_empty())
        .ok_or_else(|| "Authenticated session is missing the user name.".to_string())?;
    Ok(Some(AuthSession {
        user_name,
        is_admin: response.is_admin,
    }))
}

pub(crate) fn registration_route_access(
    session: Option<&AuthSession>,
    bootstrap_registration_open: bool,
) -> RegistrationRouteAccess {
    if session.is_some_and(|session| session.is_admin) {
        RegistrationRouteAccess::Register
    } else if session.is_some() {
        RegistrationRouteAccess::Unavailable
    } else if bootstrap_registration_open {
        RegistrationRouteAccess::Register
    } else {
        RegistrationRouteAccess::SignIn
    }
}

pub(crate) fn sign_in_shows_registration_link(bootstrap_registration_open: bool) -> bool {
    bootstrap_registration_open
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_session_from_response_requires_a_user_name_when_authenticated() {
        assert_eq!(
            auth_session_from_response(AuthSessionResponse {
                authenticated: true,
                is_admin: false,
                bootstrap_registration_open: false,
                user_name: None,
            })
            .unwrap_err(),
            "Authenticated session is missing the user name."
        );
    }

    #[test]
    fn registration_route_access_is_bootstrap_or_admin_only() {
        let admin = AuthSession {
            user_name: "alice".to_string(),
            is_admin: true,
        };
        let non_admin = AuthSession {
            user_name: "bob".to_string(),
            is_admin: false,
        };

        assert_eq!(
            registration_route_access(None, true),
            RegistrationRouteAccess::Register
        );
        assert_eq!(
            registration_route_access(None, false),
            RegistrationRouteAccess::SignIn
        );
        assert_eq!(
            registration_route_access(Some(&admin), false),
            RegistrationRouteAccess::Register
        );
        assert_eq!(
            registration_route_access(Some(&non_admin), false),
            RegistrationRouteAccess::Unavailable
        );
    }
}
