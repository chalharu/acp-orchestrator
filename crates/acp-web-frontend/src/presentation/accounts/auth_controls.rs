#![cfg_attr(not(target_family = "wasm"), allow(dead_code))]

use leptos::prelude::*;

#[cfg(target_family = "wasm")]
use crate::infrastructure::api;
use crate::routing::app_session_path;

use super::super::{AppIcon, app_icon_view, workspaces_path_with_return_to};
use super::shared::{
    accounts_path_with_return_to, sign_in_path_with_return_to, sign_out_button_label,
    sign_out_handler,
};

#[derive(Clone)]
struct SessionSidebarAuthViewState {
    accounts_href: String,
    workspaces_href: String,
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
    let sign_in_href = sign_in_path_with_return_to(&app_session_path(&current_session_id));
    let view_state = SessionSidebarAuthViewState {
        accounts_href: accounts_path_with_return_to(&app_session_path(&current_session_id)),
        workspaces_href: workspaces_path_with_return_to(&app_session_path(&current_session_id)),
        is_admin,
        signed_in,
        signing_out,
    };
    let sign_out = sign_out_handler(error, signing_out, sign_in_href);

    initialize_session_sidebar_auth_controls(checked, signed_in, is_admin, error);

    session_sidebar_auth_controls_view(view_state, sign_out)
}

#[cfg(target_family = "wasm")]
fn session_sidebar_auth_controls_view(
    state: SessionSidebarAuthViewState,
    sign_out: Callback<web_sys::MouseEvent>,
) -> impl IntoView {
    let accounts_href = state.accounts_href;
    let workspaces_href = state.workspaces_href;
    let is_admin = state.is_admin;
    let signed_in = state.signed_in;
    let signing_out = state.signing_out;

    view! {
        {move || session_sidebar_workspaces_link_view(&workspaces_href, signed_in.get())}
        {move || session_sidebar_accounts_link_view(&accounts_href, is_admin.get())}
        {move || session_sidebar_sign_out_button_view(sign_out, signed_in.get(), signing_out.get())}
    }
}

#[cfg(not(target_family = "wasm"))]
fn session_sidebar_auth_controls_view(
    state: SessionSidebarAuthViewState,
    sign_out: Callback<web_sys::MouseEvent>,
) -> impl IntoView {
    let workspaces_link = session_sidebar_workspaces_link_view(
        &state.workspaces_href,
        state.signed_in.get_untracked(),
    );
    let accounts_link =
        session_sidebar_accounts_link_view(&state.accounts_href, state.is_admin.get_untracked());
    let sign_out_button = session_sidebar_sign_out_button_view(
        sign_out,
        state.signed_in.get_untracked(),
        state.signing_out.get_untracked(),
    );

    (workspaces_link, accounts_link, sign_out_button)
}

fn session_sidebar_workspaces_link_view(workspaces_href: &str, signed_in: bool) -> AnyView {
    if signed_in {
        session_sidebar_icon_link_view(workspaces_href, "Workspaces", AppIcon::Workspaces)
    } else {
        ().into_any()
    }
}

fn session_sidebar_accounts_link_view(accounts_href: &str, is_admin: bool) -> AnyView {
    if is_admin {
        session_sidebar_icon_link_view(accounts_href, "Accounts", AppIcon::Accounts)
    } else {
        ().into_any()
    }
}

fn session_sidebar_icon_link_view(href: &str, label: &'static str, icon: AppIcon) -> AnyView {
    view! {
        <a
            class="session-sidebar__secondary-link session-sidebar__secondary-icon-link icon-action icon-action--ghost"
            href=href.to_string()
            aria-label=label
            title=label
        >
            <span class="session-sidebar__secondary-link-icon" aria-hidden="true">
                {app_icon_view(icon)}
            </span>
            <span class="sr-only">{label}</span>
        </a>
    }
    .into_any()
}

fn session_sidebar_sign_out_button_view(
    sign_out: Callback<web_sys::MouseEvent>,
    signed_in: bool,
    signing_out: bool,
) -> AnyView {
    if !signed_in {
        return ().into_any();
    }

    view! {
        <button
            type="button"
            class="session-sidebar__secondary-link session-sidebar__secondary-button session-sidebar__secondary-icon-link icon-action icon-action--ghost"
            prop:disabled=signing_out
            on:click=move |event| sign_out.run(event)
            aria-label=sign_out_button_label(signing_out)
            title=sign_out_button_label(signing_out)
        >
            <span class="session-sidebar__secondary-link-icon" aria-hidden="true">
                {app_icon_view(if signing_out {
                    AppIcon::Busy
                } else {
                    AppIcon::SignOut
                })}
            </span>
            <span class="sr-only">{sign_out_button_label(signing_out)}</span>
        </button>
    }
    .into_any()
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
                    workspaces_href: "/app/workspaces/?return_to=%2Fapp%2Fsessions%2Fabc"
                        .to_string(),
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
                    workspaces_href: "/app/workspaces/".to_string(),
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
                    workspaces_href: "/app/workspaces/".to_string(),
                    is_admin: RwSignal::new(false),
                    signed_in: RwSignal::new(false),
                    signing_out: RwSignal::new(false),
                },
                Callback::new(|_: web_sys::MouseEvent| {}),
            );
        });
    }

    #[test]
    fn sidebar_link_helpers_cover_signed_in_and_admin_variants() {
        let owner = Owner::new();
        owner.with(|| {
            let sign_out = Callback::new(|_: web_sys::MouseEvent| {});

            let _ = session_sidebar_workspaces_link_view("/app/workspaces/", true);
            let _ = session_sidebar_workspaces_link_view("/app/workspaces/", false);
            let _ = session_sidebar_accounts_link_view("/app/accounts/", true);
            let _ = session_sidebar_accounts_link_view("/app/accounts/", false);
            let _ = session_sidebar_sign_out_button_view(sign_out, true, false);
            let _ = session_sidebar_sign_out_button_view(sign_out, true, true);
            let _ = session_sidebar_sign_out_button_view(sign_out, false, false);
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
