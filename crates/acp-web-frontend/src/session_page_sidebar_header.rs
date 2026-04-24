use leptos::prelude::*;

use crate::{
    presentation::{SessionSidebarAuthControls, workspaces_path_with_return_to},
    routing::app_session_path,
};

#[cfg(target_family = "wasm")]
#[component]
pub(super) fn SessionSidebarHeader(
    current_session_id: String,
    auth_error: RwSignal<Option<String>>,
    sidebar_open: RwSignal<bool>,
) -> impl IntoView {
    let workspaces_href = workspaces_path_with_return_to(&app_session_path(&current_session_id));

    view! {
        <div class="session-sidebar__header">
            <div class="session-sidebar__header-links">
                <a class="session-sidebar__new-link" href=workspaces_href aria-label="Workspaces">
                    <span class="session-sidebar__new-link-icon" aria-hidden="true">
                        "+"
                    </span>
                    <span class="session-sidebar__new-link-label">"Workspaces"</span>
                </a>
                <SessionSidebarAuthControls current_session_id=current_session_id error=auth_error />
            </div>
            <button
                type="button"
                class="session-sidebar__dismiss"
                on:click=move |_| sidebar_open.set(false)
            >
                <span aria-hidden="true">{"✕"}</span>
                <span class="sr-only">"Close sidebar"</span>
            </button>
        </div>
    }
}

#[cfg(not(target_family = "wasm"))]
#[component]
pub(super) fn SessionSidebarHeader(
    current_session_id: String,
    auth_error: RwSignal<Option<String>>,
    sidebar_open: RwSignal<bool>,
) -> impl IntoView {
    let _ = sidebar_open;
    let workspaces_href = workspaces_path_with_return_to(&app_session_path(&current_session_id));

    view! {
        <div class="session-sidebar__header">
            <div class="session-sidebar__header-links">
                <a class="session-sidebar__new-link" href=workspaces_href aria-label="Workspaces">
                    <span class="session-sidebar__new-link-icon" aria-hidden="true">
                        "+"
                    </span>
                    <span class="session-sidebar__new-link-label">"Workspaces"</span>
                </a>
                <SessionSidebarAuthControls current_session_id=current_session_id error=auth_error />
            </div>
            <button type="button" class="session-sidebar__dismiss">
                <span aria-hidden="true">{"✕"}</span>
                <span class="sr-only">"Close sidebar"</span>
            </button>
        </div>
    }
}
