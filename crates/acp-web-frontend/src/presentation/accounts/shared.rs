#![cfg_attr(not(target_family = "wasm"), allow(dead_code))]

use acp_contracts_accounts::LocalAccount;
use leptos::prelude::*;
#[cfg(target_family = "wasm")]
use wasm_bindgen::JsCast;

use crate::{
    application::auth::AccountsRouteAccess,
    infrastructure::api,
    routing::{AppRoute, route_from_pathname},
};

#[derive(Clone, Copy)]
pub(super) struct AccountsPageState {
    pub(super) error: RwSignal<Option<String>>,
    pub(super) notice: RwSignal<Option<String>>,
    pub(super) access: RwSignal<Option<AccountsRouteAccess>>,
    pub(super) current_user_id: RwSignal<String>,
    pub(super) accounts: RwSignal<Vec<LocalAccount>>,
    pub(super) loading_accounts: RwSignal<bool>,
    pub(super) create_username: RwSignal<String>,
    pub(super) create_password: RwSignal<String>,
    pub(super) create_admin: RwSignal<bool>,
    pub(super) creating: RwSignal<bool>,
    pub(super) checked: RwSignal<bool>,
}

impl AccountsPageState {
    pub(super) fn new() -> Self {
        Self {
            error: RwSignal::new(None::<String>),
            notice: RwSignal::new(None::<String>),
            access: RwSignal::new(None::<AccountsRouteAccess>),
            current_user_id: RwSignal::new(String::new()),
            accounts: RwSignal::new(Vec::<LocalAccount>::new()),
            loading_accounts: RwSignal::new(true),
            create_username: RwSignal::new(String::new()),
            create_password: RwSignal::new(String::new()),
            create_admin: RwSignal::new(false),
            creating: RwSignal::new(false),
            checked: RwSignal::new(false),
        }
    }
}

#[cfg(target_family = "wasm")]
pub(super) fn initialize_accounts_page(state: AccountsPageState) {
    Effect::new(move |_| {
        if state.checked.get() {
            return;
        }

        state.checked.set(true);
        leptos::task::spawn_local(async move {
            match api::auth_status().await {
                Ok(status) => {
                    let access = crate::application::auth::accounts_route_access(&status);
                    let should_load_accounts = matches!(access, AccountsRouteAccess::Admin(_));
                    state.access.set(Some(access));
                    if should_load_accounts {
                        spawn_account_reload(state);
                    } else {
                        state.loading_accounts.set(false);
                    }
                }
                Err(message) => {
                    state.loading_accounts.set(false);
                    state.error.set(Some(message));
                }
            }
        });
    });
}

#[cfg(not(target_family = "wasm"))]
pub(super) fn initialize_accounts_page(state: AccountsPageState) {
    initialize_accounts_page_host(state);
}

#[cfg(not(target_family = "wasm"))]
pub(super) fn initialize_accounts_page_host(state: AccountsPageState) {
    if state.checked.get_untracked() {
        return;
    }

    state.checked.set(true);
    state.loading_accounts.set(false);
}

pub(super) fn accounts_path_with_return_to(return_to_path: &str) -> String {
    format!(
        "/app/accounts/?return_to={}",
        api::encode_component(return_to_path)
    )
}

#[cfg(target_family = "wasm")]
pub(super) fn accounts_back_to_chat_path_from_location() -> String {
    web_sys::window()
        .and_then(|window| window.location().search().ok())
        .map(|search| accounts_back_to_chat_path(&search))
        .unwrap_or_else(|| "/app/".to_string())
}

#[cfg(not(target_family = "wasm"))]
pub(super) fn accounts_back_to_chat_path_from_location() -> String {
    "/app/".to_string()
}

fn accounts_back_to_chat_path(search: &str) -> String {
    query_param(search, "return_to")
        .filter(|path| matches!(route_from_pathname(path), AppRoute::Session(_)))
        .unwrap_or_else(|| "/app/".to_string())
}

fn query_param(search: &str, name: &str) -> Option<String> {
    search
        .trim_start_matches('?')
        .split('&')
        .filter(|pair| !pair.is_empty())
        .find_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            (key == name)
                .then(|| api::decode_component(value))
                .flatten()
        })
}

pub(super) fn accounts_page_shows_sign_out(access: Option<AccountsRouteAccess>) -> bool {
    matches!(
        access,
        Some(AccountsRouteAccess::Admin(_)) | Some(AccountsRouteAccess::Forbidden)
    )
}

#[cfg(target_family = "wasm")]
pub(super) fn sign_out_handler(
    error: RwSignal<Option<String>>,
    signing_out: RwSignal<bool>,
) -> Callback<web_sys::MouseEvent> {
    Callback::new(move |_event: web_sys::MouseEvent| {
        if signing_out.get_untracked() {
            return;
        }

        signing_out.set(true);
        error.set(None);
        leptos::task::spawn_local(async move {
            match api::sign_out().await {
                Ok(()) => {
                    crate::browser::clear_prepared_session_id();
                    if let Err(message) = crate::browser::navigate_to("/app/sign-in/") {
                        signing_out.set(false);
                        error.set(Some(message));
                    }
                }
                Err(message) => {
                    signing_out.set(false);
                    error.set(Some(message));
                }
            }
        });
    })
}

#[cfg(not(target_family = "wasm"))]
pub(super) fn sign_out_handler(
    error: RwSignal<Option<String>>,
    signing_out: RwSignal<bool>,
) -> Callback<web_sys::MouseEvent> {
    Callback::new(move |_event: web_sys::MouseEvent| sign_out_host(error, signing_out))
}

pub(super) fn sign_out_button_label(signing_out: bool) -> &'static str {
    if signing_out {
        "Signing out…"
    } else {
        "Sign out"
    }
}

#[cfg(target_family = "wasm")]
pub(super) fn event_target_checked(event: &web_sys::Event) -> bool {
    event
        .target()
        .and_then(|target| target.dyn_into::<web_sys::HtmlInputElement>().ok())
        .map(|input| input.checked())
        .unwrap_or(false)
}

#[cfg(not(target_family = "wasm"))]
pub(super) fn event_target_checked<T>(_event: &T) -> bool {
    false
}

#[cfg(not(target_family = "wasm"))]
pub(super) fn sign_out_host(error: RwSignal<Option<String>>, signing_out: RwSignal<bool>) {
    if signing_out.get_untracked() {
        return;
    }

    signing_out.set(true);
    error.set(None);
}

#[cfg(target_family = "wasm")]
pub(super) fn spawn_account_reload(state: AccountsPageState) {
    state.loading_accounts.set(true);
    state.error.set(None);
    leptos::task::spawn_local(async move {
        match api::list_accounts().await {
            Ok(response) => {
                state.current_user_id.set(response.current_user_id);
                state.accounts.set(response.accounts);
                state.loading_accounts.set(false);
            }
            Err(message) => {
                state.loading_accounts.set(false);
                state.error.set(Some(message));
            }
        }
    });
}

#[cfg(not(target_family = "wasm"))]
pub(super) fn spawn_account_reload(state: AccountsPageState) {
    state.loading_accounts.set(true);
    state.error.set(None);
    state.loading_accounts.set(false);
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use leptos::prelude::*;

    use super::*;
    use wasm_bindgen::{JsCast, JsValue};

    fn sample_account(user_id: &str, is_admin: bool) -> LocalAccount {
        LocalAccount {
            user_id: user_id.to_string(),
            username: user_id.to_string(),
            is_admin,
            created_at: Utc.with_ymd_and_hms(2026, 4, 17, 1, 0, 0).unwrap(),
        }
    }

    #[test]
    fn accounts_paths_preserve_only_session_routes() {
        assert_eq!(
            accounts_path_with_return_to("/app/sessions/s%2F1"),
            "/app/accounts/?return_to=%2Fapp%2Fsessions%2Fs%252F1"
        );
        assert_eq!(
            accounts_back_to_chat_path("?return_to=%2Fapp%2Fsessions%2Fs%252F1"),
            "/app/sessions/s%2F1"
        );
        assert_eq!(accounts_back_to_chat_path("?return_to=%2Fapp%2F"), "/app/");
        assert_eq!(
            accounts_back_to_chat_path("?return_to=https%3A%2F%2Fexample.com"),
            "/app/"
        );
    }

    #[test]
    fn query_param_and_sign_out_visibility_helpers_match_accounts_routes() {
        assert_eq!(
            query_param("?return_to=%2Fapp%2Fsessions%2Fabc&x=1", "return_to"),
            Some("/app/sessions/abc".to_string())
        );
        assert_eq!(query_param("?x=1", "return_to"), None);
        assert!(accounts_page_shows_sign_out(Some(
            AccountsRouteAccess::Forbidden
        )));
        assert!(!accounts_page_shows_sign_out(Some(
            AccountsRouteAccess::SignInRequired
        )));
        assert_eq!(sign_out_button_label(false), "Sign out");
        assert_eq!(sign_out_button_label(true), "Signing out…");
    }

    #[test]
    fn accounts_page_shows_sign_out_for_admin_and_none() {
        let admin = sample_account("admin", true);
        assert!(accounts_page_shows_sign_out(Some(
            AccountsRouteAccess::Admin(admin)
        )));
        assert!(!accounts_page_shows_sign_out(None));
        assert!(!accounts_page_shows_sign_out(Some(
            AccountsRouteAccess::RegisterRequired
        )));
    }

    #[test]
    fn query_param_returns_none_for_missing_key_and_empty_search() {
        assert_eq!(query_param("", "return_to"), None);
        assert_eq!(query_param("?a=1&b=2", "return_to"), None);
        assert_eq!(
            query_param("return_to=%2Fapp%2F", "return_to"),
            Some("/app/".to_string())
        );
    }

    #[test]
    fn accounts_page_state_starts_with_empty_defaults() {
        let owner = Owner::new();
        owner.with(|| {
            let state = AccountsPageState::new();
            assert!(state.error.get().is_none());
            assert!(state.notice.get().is_none());
            assert!(state.access.get().is_none());
            assert!(state.current_user_id.get().is_empty());
            assert!(state.accounts.get().is_empty());
            assert!(state.loading_accounts.get());
            assert!(state.create_username.get().is_empty());
            assert!(state.create_password.get().is_empty());
            assert!(!state.create_admin.get());
            assert!(!state.creating.get());
            assert!(!state.checked.get());
        });
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn host_initializers_mark_state_once() {
        let owner = Owner::new();
        owner.with(|| {
            let state = AccountsPageState::new();
            initialize_accounts_page_host(state);
            assert!(state.checked.get());
            assert!(!state.loading_accounts.get());
            state.loading_accounts.set(true);
            initialize_accounts_page_host(state);
            assert!(state.loading_accounts.get());
        });
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn sign_out_and_reload_host_clear_errors() {
        let owner = Owner::new();
        owner.with(|| {
            let state = AccountsPageState::new();
            state.error.set(Some("stale error".to_string()));
            let signing_out = RwSignal::new(false);
            sign_out_host(state.error, signing_out);
            assert!(signing_out.get());
            assert!(state.error.get().is_none());

            state.error.set(Some("reload error".to_string()));
            state.loading_accounts.set(true);
            spawn_account_reload(state);
            assert!(!state.loading_accounts.get());
            assert!(state.error.get().is_none());
        });
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn accounts_back_to_chat_path_defaults_to_app_root() {
        assert_eq!(accounts_back_to_chat_path_from_location(), "/app/");
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn event_target_checked_returns_false_on_host() {
        assert!(!super::event_target_checked(&()));
    }

    #[cfg(not(target_family = "wasm"))]
    fn fake_mouse_event() -> web_sys::MouseEvent {
        JsValue::NULL.unchecked_into()
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn host_sign_out_handler_leaves_in_progress_state_unchanged() {
        let owner = Owner::new();
        owner.with(|| {
            let state = AccountsPageState::new();
            state.error.set(Some("still signing out".to_string()));
            let signing_out = RwSignal::new(true);
            sign_out_host(state.error, signing_out);
            assert_eq!(state.error.get(), Some("still signing out".to_string()));
            sign_out_handler(state.error, RwSignal::new(false)).run(fake_mouse_event());
        });
    }
}
