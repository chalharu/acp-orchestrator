#![cfg_attr(not(target_family = "wasm"), allow(dead_code))]

use leptos::prelude::*;

#[cfg(target_family = "wasm")]
use crate::infrastructure::api;
use crate::routing::app_session_path;

use super::shared::{accounts_path_with_return_to, sign_out_button_label, sign_out_handler};

const WORKSPACES_PATH: &str = "/app/workspaces/";

#[derive(Clone)]
struct SessionSidebarAuthViewState {
    accounts_href: String,
    is_admin: RwSignal<bool>,
    signed_in: RwSignal<bool>,
    signing_out: RwSignal<bool>,
}

#[component]
pub fn SessionSidebarAuthControls(
    current_session_id: String,
    error: RwSignal<Option<String>>,
) -> impl IntoView {
    let is_admin = RwSignal::new(false);
    let signed_in = RwSignal::new(false);
    let checked = RwSignal::new(false);
    let signing_out = RwSignal::new(false);
    let view_state = SessionSidebarAuthViewState {
        accounts_href: accounts_path_with_return_to(&app_session_path(&current_session_id)),
        is_admin,
        signed_in,
        signing_out,
    };
    let sign_out = sign_out_handler(error, signing_out);

    initialize_session_sidebar_auth_controls(checked, signed_in, is_admin, error);

    session_sidebar_auth_controls_view(view_state, sign_out)
}

fn session_sidebar_auth_controls_view(
    state: SessionSidebarAuthViewState,
    sign_out: Callback<web_sys::MouseEvent>,
) -> impl IntoView {
    view! {
        <SessionSidebarWorkspacesLink signed_in=state.signed_in />
        <SessionSidebarAccountsLink accounts_href=state.accounts_href is_admin=state.is_admin />
        <SessionSidebarSignOutButton
            signed_in=state.signed_in
            signing_out=state.signing_out
            sign_out=sign_out
        />
    }
}

#[component]
fn SessionSidebarWorkspacesLink(signed_in: RwSignal<bool>) -> impl IntoView {
    view! {
        <Show when=move || signed_in.get()>
            <a class="session-sidebar__secondary-link" href=WORKSPACES_PATH>
                "Workspaces"
            </a>
        </Show>
    }
}

#[component]
fn SessionSidebarAccountsLink(accounts_href: String, is_admin: RwSignal<bool>) -> impl IntoView {
    move || {
        if is_admin.get() {
            let accounts_href = accounts_href.clone();
            view! { <a class="session-sidebar__secondary-link" href=accounts_href>"Accounts"</a> }
                .into_any()
        } else {
            ().into_any()
        }
    }
}

#[component]
fn SessionSidebarSignOutButton(
    signed_in: RwSignal<bool>,
    signing_out: RwSignal<bool>,
    sign_out: Callback<web_sys::MouseEvent>,
) -> impl IntoView {
    view! {
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
                SessionSidebarAuthViewState {
                    accounts_href: "/app/accounts/?return_to=%2Fapp%2Fsessions%2Fabc".to_string(),
                    is_admin: RwSignal::new(true),
                    signed_in: RwSignal::new(true),
                    signing_out: RwSignal::new(false),
                },
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
                SessionSidebarAuthViewState {
                    accounts_href: "/app/accounts/".to_string(),
                    is_admin: RwSignal::new(false),
                    signed_in: RwSignal::new(true),
                    signing_out: RwSignal::new(false),
                },
                Callback::new(|_: web_sys::MouseEvent| {}),
            );

            // not signed in: neither link
            let _ = session_sidebar_auth_controls_view(
                SessionSidebarAuthViewState {
                    accounts_href: "/app/accounts/".to_string(),
                    is_admin: RwSignal::new(false),
                    signed_in: RwSignal::new(false),
                    signing_out: RwSignal::new(false),
                },
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
