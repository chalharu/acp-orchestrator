use leptos::prelude::*;

use crate::presentation::SessionSidebarAuthControls;

#[cfg(target_family = "wasm")]
#[component]
pub(super) fn SessionSidebarHeader(
    current_session_id: String,
    auth_error: RwSignal<Option<String>>,
    sidebar_open: RwSignal<bool>,
) -> impl IntoView {
    view! {
        <div class="session-sidebar__header">
            <div class="session-sidebar__header-links">
                <a class="session-sidebar__new-link" href="/app/" aria-label="New chat">
                    <span class="session-sidebar__new-link-icon" aria-hidden="true">
                        "+"
                    </span>
                    <span class="session-sidebar__new-link-label">"New chat"</span>
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

    view! {
        <div class="session-sidebar__header">
            <div class="session-sidebar__header-links">
                <a class="session-sidebar__new-link" href="/app/" aria-label="New chat">
                    <span class="session-sidebar__new-link-icon" aria-hidden="true">
                        "+"
                    </span>
                    <span class="session-sidebar__new-link-label">"New chat"</span>
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
