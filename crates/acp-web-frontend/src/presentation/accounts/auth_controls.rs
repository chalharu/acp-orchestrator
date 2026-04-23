#![cfg_attr(not(target_family = "wasm"), allow(dead_code))]

use leptos::prelude::*;

#[cfg(target_family = "wasm")]
use crate::infrastructure::api;
use crate::routing::app_session_path;

use super::shared::{accounts_path_with_return_to, sign_out_button_label, sign_out_handler};

const WORKSPACES_PATH: &str = "/app/workspaces/";

#[component]
pub fn SessionSidebarAuthControls(
    current_session_id: String,
    error: RwSignal<Option<String>>,
) -> impl IntoView {
    let is_admin = RwSignal::new(false);
    let signed_in = RwSignal::new(false);
    let checked = RwSignal::new(false);
    let signing_out = RwSignal::new(false);
    let accounts_href = accounts_path_with_return_to(&app_session_path(&current_session_id));
    let sign_out = sign_out_handler(error, signing_out);

    initialize_session_sidebar_auth_controls(checked, signed_in, is_admin, error);

    session_sidebar_auth_controls_view(accounts_href, is_admin, signed_in, signing_out, sign_out)
}

#[cfg(target_family = "wasm")]
fn session_sidebar_auth_controls_view(
    accounts_href: String,
    is_admin: RwSignal<bool>,
    signed_in: RwSignal<bool>,
    signing_out: RwSignal<bool>,
    sign_out: Callback<web_sys::MouseEvent>,
) -> impl IntoView {
    view! {
        <Show when=move || signed_in.get()>
            <a class="session-sidebar__secondary-link" href=WORKSPACES_PATH>
                "Workspaces"
            </a>
        </Show>
        <Show when=move || is_admin.get()>
            <a class="session-sidebar__secondary-link" href=accounts_href.clone()>
                "Accounts"
            </a>
        </Show>
        <Show when=move || signed_in.get()>
            <button
                type="button"
                class="session-sidebar__secondary-link session-sidebar__secondary-button"
                prop:disabled=move || signing_out.get()
                on:click=move |event| sign_out.run(event)
            >
                {move || sign_out_button_label(signing_out.get())}
            </button>
        </Show>
    }
}

#[cfg(not(target_family = "wasm"))]
fn session_sidebar_auth_controls_view(
    accounts_href: String,
    is_admin: RwSignal<bool>,
    signed_in: RwSignal<bool>,
    signing_out: RwSignal<bool>,
    sign_out: Callback<web_sys::MouseEvent>,
) -> impl IntoView {
    let signed_in_val = signed_in.get_untracked();
    let is_admin_val = is_admin.get_untracked();
    let signing_out_val = signing_out.get_untracked();

    let workspaces_link = if signed_in_val {
        view! {
            <a class="session-sidebar__secondary-link" href=WORKSPACES_PATH>
                "Workspaces"
            </a>
        }
        .into_any()
    } else {
        ().into_any()
    };

    let accounts_link = if is_admin_val {
        view! {
            <a class="session-sidebar__secondary-link" href=accounts_href>
                "Accounts"
            </a>
        }
        .into_any()
    } else {
        ().into_any()
    };

    let sign_out_button = if signed_in_val {
        let label = sign_out_button_label(signing_out_val);
        view! {
            <button
                type="button"
                class="session-sidebar__secondary-link session-sidebar__secondary-button"
                prop:disabled=signing_out_val
                on:click=move |event| sign_out.run(event)
            >
                {label}
            </button>
        }
        .into_any()
    } else {
        ().into_any()
    };

    view! {
        {workspaces_link}
        {accounts_link}
        {sign_out_button}
    }
}

#[cfg(target_family = "wasm")]
fn initialize_session_sidebar_auth_controls(
    checked: RwSignal<bool>,
    signed_in: RwSignal<bool>,
    is_admin: RwSignal<bool>,
    error: RwSignal<Option<String>>,
) {
    Effect::new(move |_| {
        if checked.get() {
            return;
        }
        checked.set(true);
        leptos::task::spawn_local(async move {
            match api::auth_status().await {
                Ok(status) => {
                    signed_in.set(status.account.is_some());
                    is_admin.set(status.account.is_some_and(|account| account.is_admin));
                }
                Err(message) => error.set(Some(message)),
            }
        });
    });
}

#[cfg(not(target_family = "wasm"))]
fn initialize_session_sidebar_auth_controls(
    checked: RwSignal<bool>,
    _signed_in: RwSignal<bool>,
    _is_admin: RwSignal<bool>,
    _error: RwSignal<Option<String>>,
) {
    initialize_session_sidebar_auth_controls_host(checked);
}

#[cfg(not(target_family = "wasm"))]
fn initialize_session_sidebar_auth_controls_host(checked: RwSignal<bool>) {
    if checked.get_untracked() {
        return;
    }

    checked.set(true);
}

#[cfg(test)]
mod tests {
    use leptos::prelude::*;

    use super::*;

    #[test]
    fn session_sidebar_auth_controls_and_helper_views_render_host_safe_views() {
        let owner = Owner::new();
        owner.with(|| {
            let error = RwSignal::new(None::<String>);
            let _ = view! {
                <SessionSidebarAuthControls current_session_id="session-1".to_string() error=error />
            };

            let _ = session_sidebar_auth_controls_view(
                "/app/accounts/?return_to=%2Fapp%2Fsessions%2Fabc".to_string(),
                RwSignal::new(true),
                RwSignal::new(true),
                RwSignal::new(false),
                Callback::new(|_: web_sys::MouseEvent| {}),
            );
        });
    }

    #[test]
    fn workspaces_link_appears_for_signed_in_and_accounts_for_admin() {
        let owner = Owner::new();
        owner.with(|| {
            // signed-in non-admin: workspaces visible, accounts hidden
            let _ = session_sidebar_auth_controls_view(
                "/app/accounts/".to_string(),
                RwSignal::new(false), // not admin
                RwSignal::new(true),  // signed in
                RwSignal::new(false),
                Callback::new(|_: web_sys::MouseEvent| {}),
            );

            // not signed in: neither link
            let _ = session_sidebar_auth_controls_view(
                "/app/accounts/".to_string(),
                RwSignal::new(false),
                RwSignal::new(false),
                RwSignal::new(false),
                Callback::new(|_: web_sys::MouseEvent| {}),
            );
        });
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn host_initializer_marks_state_once() {
        let owner = Owner::new();
        owner.with(|| {
            let checked = RwSignal::new(false);
            initialize_session_sidebar_auth_controls_host(checked);
            assert!(checked.get());
            initialize_session_sidebar_auth_controls_host(checked);
            assert!(checked.get());
        });
    }
}
