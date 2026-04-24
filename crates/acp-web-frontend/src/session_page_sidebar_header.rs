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
    session_sidebar_header_view(
        current_session_id,
        auth_error,
        view! {
            <button
                type="button"
                class="session-sidebar__dismiss"
                on:click=move |_| sidebar_open.set(false)
            >
                <span aria-hidden="true">{"✕"}</span>
                <span class="sr-only">"Close sidebar"</span>
            </button>
        },
    )
}

#[cfg(not(target_family = "wasm"))]
#[component]
pub(super) fn SessionSidebarHeader(
    current_session_id: String,
    auth_error: RwSignal<Option<String>>,
    sidebar_open: RwSignal<bool>,
) -> impl IntoView {
    let _ = sidebar_open;
    session_sidebar_header_view(
        current_session_id,
        auth_error,
        view! {
            <button type="button" class="session-sidebar__dismiss">
                <span aria-hidden="true">{"✕"}</span>
                <span class="sr-only">"Close sidebar"</span>
            </button>
        },
    )
}

fn session_sidebar_header_view(
    current_session_id: String,
    auth_error: RwSignal<Option<String>>,
    dismiss_button: impl IntoView + 'static,
) -> impl IntoView {
    let workspaces_href = session_sidebar_workspaces_path(&current_session_id);

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
            {dismiss_button}
        </div>
    }
}

fn session_sidebar_workspaces_path(current_session_id: &str) -> String {
    workspaces_path_with_return_to(&app_session_path(current_session_id))
}

#[cfg(test)]
mod tests {
    use leptos::prelude::*;

    use super::*;

    #[test]
    fn session_sidebar_header_builds_without_panicking() {
        let owner = Owner::new();
        owner.with(|| {
            let auth_error = RwSignal::new(None::<String>);
            let sidebar_open = RwSignal::new(true);
            let _ = view! {
                <SessionSidebarHeader
                    current_session_id="session-1".to_string()
                    auth_error=auth_error
                    sidebar_open=sidebar_open
                />
            };
        });
    }

    #[test]
    fn session_sidebar_workspaces_path_preserves_current_session_return_to() {
        assert_eq!(
            session_sidebar_workspaces_path("s/1"),
            "/app/workspaces/?return_to=%2Fapp%2Fsessions%2Fs%252F1"
        );
    }
}
