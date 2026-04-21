use acp_contracts::AuthSessionResponse;
use leptos::prelude::*;

use crate::{
    api,
    domain::auth::{AuthSession, auth_session_from_response, registration_route_access},
};

#[derive(Clone, Copy)]
pub(crate) struct AuthSignals {
    pub(crate) session: RwSignal<Option<AuthSession>>,
    pub(crate) checked: RwSignal<bool>,
    pub(crate) signing_in: RwSignal<bool>,
    pub(crate) signing_up: RwSignal<bool>,
    pub(crate) bootstrap_registration_open: RwSignal<bool>,
    pub(crate) registration_notice: RwSignal<Option<String>>,
    pub(crate) error: RwSignal<Option<String>>,
    pub(crate) user_name_draft: RwSignal<String>,
    pub(crate) password_draft: RwSignal<String>,
}

pub(crate) fn auth_signals() -> AuthSignals {
    AuthSignals {
        session: RwSignal::new(None::<AuthSession>),
        checked: RwSignal::new(false),
        signing_in: RwSignal::new(false),
        signing_up: RwSignal::new(false),
        bootstrap_registration_open: RwSignal::new(false),
        registration_notice: RwSignal::new(None::<String>),
        error: RwSignal::new(None::<String>),
        user_name_draft: RwSignal::new(String::new()),
        password_draft: RwSignal::new(String::new()),
    }
}

pub(crate) fn load_auth_session_once(auth: AuthSignals) {
    let started = RwSignal::new(false);
    Effect::new(move |_| {
        if started.get() {
            return;
        }

        started.set(true);
        spawn_auth_session_load(auth);
    });
}

pub(crate) fn spawn_auth_session_load(auth: AuthSignals) {
    leptos::task::spawn_local(async move {
        match api::load_auth_session().await {
            Ok(response) => {
                if let Err(message) = apply_auth_session_response(auth, response) {
                    set_auth_error(auth, message);
                }
            }
            Err(message) => set_auth_error(auth, message),
        }
    });
}

pub(crate) fn submit_sign_in(auth: AuthSignals) {
    let user_name = auth.user_name_draft.get_untracked().trim().to_string();
    let password = auth.password_draft.get_untracked();
    if user_name.is_empty() {
        auth.error
            .set(Some("Enter a user name to continue.".to_string()));
        return;
    }
    if password.is_empty() {
        auth.error
            .set(Some("Enter a password to continue.".to_string()));
        return;
    }
    if auth.signing_in.get_untracked() || auth.signing_up.get_untracked() {
        return;
    }

    auth.signing_in.set(true);
    auth.error.set(None);
    auth.registration_notice.set(None);
    leptos::task::spawn_local(async move {
        match api::sign_in(&user_name, &password).await {
            Ok(response) => match apply_auth_session_response(auth, response) {
                Ok(Some(_)) => clear_auth_drafts(auth),
                Ok(None) => auth.error.set(Some(
                    "Sign in did not establish an authenticated session.".to_string(),
                )),
                Err(message) => set_auth_error(auth, message),
            },
            Err(message) => set_auth_error(auth, message),
        }
        auth.signing_in.set(false);
    });
}

pub(crate) fn submit_sign_up(auth: AuthSignals) {
    let user_name = auth.user_name_draft.get_untracked().trim().to_string();
    let password = auth.password_draft.get_untracked();
    if user_name.is_empty() {
        auth.error
            .set(Some("Enter a user name to continue.".to_string()));
        return;
    }
    if password.chars().count() < 8 {
        auth.error.set(Some(
            "Enter a password with at least 8 characters.".to_string(),
        ));
        return;
    }
    if auth.signing_in.get_untracked() || auth.signing_up.get_untracked() {
        return;
    }
    if !matches!(
        registration_route_access(
            auth.session.get_untracked().as_ref(),
            auth.bootstrap_registration_open.get_untracked(),
        ),
        crate::domain::auth::RegistrationRouteAccess::Register
    ) {
        auth.error
            .set(Some("Account creation is not available.".to_string()));
        return;
    }

    let was_authenticated_admin = auth
        .session
        .get_untracked()
        .as_ref()
        .is_some_and(|session| session.is_admin);

    auth.signing_up.set(true);
    auth.error.set(None);
    auth.registration_notice.set(None);
    leptos::task::spawn_local(async move {
        match api::sign_up(&user_name, &password).await {
            Ok(response) => match apply_auth_session_response(auth, response) {
                Ok(Some(_)) => {
                    clear_auth_drafts(auth);
                    if was_authenticated_admin {
                        auth.registration_notice
                            .set(Some(format!("Created account {user_name}.")));
                    }
                }
                Ok(None) => auth.error.set(Some(
                    "Account creation did not establish an authenticated session.".to_string(),
                )),
                Err(message) => set_auth_error(auth, message),
            },
            Err(message) => set_auth_error(auth, message),
        }
        auth.signing_up.set(false);
    });
}

pub(crate) fn clear_auth_drafts(auth: AuthSignals) {
    auth.user_name_draft.set(String::new());
    auth.password_draft.set(String::new());
}

pub(crate) fn set_auth_error(auth: AuthSignals, message: String) {
    auth.error.set(Some(message));
    auth.checked.set(true);
}

pub(crate) fn apply_auth_session_response(
    auth: AuthSignals,
    response: AuthSessionResponse,
) -> Result<Option<AuthSession>, String> {
    let bootstrap_registration_open = response.bootstrap_registration_open;
    let session = auth_session_from_response(response)?;
    auth.error.set(None);
    auth.registration_notice.set(None);
    auth.bootstrap_registration_open
        .set(bootstrap_registration_open);
    auth.session.set(session.clone());
    auth.checked.set(true);
    Ok(session)
}

pub(crate) fn current_auth_user_name(auth: AuthSignals) -> Option<String> {
    auth.session
        .get_untracked()
        .map(|session| session.user_name.clone())
}
