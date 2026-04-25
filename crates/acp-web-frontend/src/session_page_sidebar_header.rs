use leptos::prelude::*;

#[cfg(target_family = "wasm")]
use crate::{
    browser::{navigate_to, store_prepared_session_id},
    infrastructure::api,
};
use crate::{
    presentation::{SessionSidebarAuthControls, workspaces_path_with_return_to},
    routing::app_session_path,
};

#[cfg(target_family = "wasm")]
#[component]
pub(super) fn SessionSidebarHeader(
    current_session_id: String,
    current_workspace_id: Signal<Option<String>>,
    auth_error: RwSignal<Option<String>>,
    sidebar_error: RwSignal<Option<String>>,
    sidebar_open: RwSignal<bool>,
) -> impl IntoView {
    session_sidebar_header_view(
        current_session_id,
        current_workspace_id,
        auth_error,
        sidebar_error,
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
    current_workspace_id: Signal<Option<String>>,
    auth_error: RwSignal<Option<String>>,
    sidebar_error: RwSignal<Option<String>>,
    sidebar_open: RwSignal<bool>,
) -> impl IntoView {
    let _ = sidebar_open;
    session_sidebar_header_view(
        current_session_id,
        current_workspace_id,
        auth_error,
        sidebar_error,
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
    current_workspace_id: Signal<Option<String>>,
    auth_error: RwSignal<Option<String>>,
    sidebar_error: RwSignal<Option<String>>,
    dismiss_button: impl IntoView + 'static,
) -> impl IntoView {
    let workspaces_href = session_sidebar_workspaces_href(&current_session_id);
    let current_session_id_for_auth = current_session_id.clone();

    #[cfg(target_family = "wasm")]
    let primary_action =
        session_sidebar_primary_action(current_workspace_id, sidebar_error, workspaces_href);
    #[cfg(not(target_family = "wasm"))]
    let primary_action = {
        let _ = sidebar_error;
        session_sidebar_primary_action(current_workspace_id, workspaces_href)
    };

    view! {
        <div class="session-sidebar__header">
            <div class="session-sidebar__header-links">
                {primary_action}
                <SessionSidebarAuthControls current_session_id=current_session_id_for_auth error=auth_error />
            </div>
            {dismiss_button}
        </div>
    }
}

fn session_sidebar_new_chat_workspace_id(current_workspace_id: Option<String>) -> Option<String> {
    current_workspace_id.filter(|workspace_id| !workspace_id.trim().is_empty())
}

fn session_sidebar_new_chat_icon(creating: bool) -> &'static str {
    if creating { "…" } else { "+" }
}

fn session_sidebar_new_chat_label(creating: bool) -> &'static str {
    if creating { "Creating…" } else { "New chat" }
}

fn session_sidebar_workspaces_icon() -> &'static str {
    "▦"
}

fn session_sidebar_workspaces_label() -> &'static str {
    "Workspaces"
}

fn session_sidebar_workspaces_href(current_session_id: &str) -> String {
    workspaces_path_with_return_to(&app_session_path(current_session_id))
}

fn session_sidebar_workspaces_link(href: &str) -> AnyView {
    view! {
        <a
            class="session-sidebar__new-link"
            href=href.to_string()
            aria-label=session_sidebar_workspaces_label()
            title=session_sidebar_workspaces_label()
        >
            <span class="session-sidebar__new-link-icon" aria-hidden="true">
                {session_sidebar_workspaces_icon()}
            </span>
            <span class="session-sidebar__new-link-label">
                {session_sidebar_workspaces_label()}
            </span>
        </a>
    }
    .into_any()
}

#[cfg(target_family = "wasm")]
fn session_sidebar_primary_action(
    current_workspace_id: Signal<Option<String>>,
    sidebar_error: RwSignal<Option<String>>,
    workspaces_href: String,
) -> AnyView {
    let can_create = Signal::derive(move || {
        session_sidebar_new_chat_workspace_id(current_workspace_id.get()).is_some()
    });

    view! {
        {move || {
            if can_create.get() {
                session_sidebar_new_chat_button(current_workspace_id, sidebar_error)
            } else {
                session_sidebar_workspaces_link(&workspaces_href)
            }
        }}
    }
    .into_any()
}

#[cfg(not(target_family = "wasm"))]
fn session_sidebar_primary_action(
    current_workspace_id: Signal<Option<String>>,
    workspaces_href: String,
) -> AnyView {
    let can_create = Signal::derive(move || {
        session_sidebar_new_chat_workspace_id(current_workspace_id.get()).is_some()
    });

    view! {
        {move || {
            if can_create.get() {
                session_sidebar_new_chat_button(current_workspace_id)
            } else {
                session_sidebar_workspaces_link(&workspaces_href)
            }
        }}
    }
    .into_any()
}

#[cfg(target_family = "wasm")]
fn session_sidebar_new_chat_button(
    current_workspace_id: Signal<Option<String>>,
    sidebar_error: RwSignal<Option<String>>,
) -> AnyView {
    let creating = RwSignal::new(false);

    view! {
        <button
            type="button"
            class="session-sidebar__new-link"
            prop:disabled=move || creating.get()
            aria-label=move || session_sidebar_new_chat_label(creating.get())
            title=move || session_sidebar_new_chat_label(creating.get())
            on:click=move |_| {
                if creating.get_untracked() {
                    return;
                }
                let Some(workspace_id) =
                    session_sidebar_new_chat_workspace_id(current_workspace_id.get_untracked())
                else {
                    sidebar_error.set(Some(
                        "Current workspace is unavailable. Open Workspaces to choose another workspace.".to_string(),
                    ));
                    return;
                };
                creating.set(true);
                sidebar_error.set(None);
                leptos::task::spawn_local(async move {
                    match api::create_workspace_session(&workspace_id).await {
                        Ok(session_id) => {
                            store_prepared_session_id(&session_id);
                            if let Err(message) = navigate_to(&app_session_path(&session_id)) {
                                creating.set(false);
                                sidebar_error.set(Some(message));
                            }
                        }
                        Err(create_error) => {
                            creating.set(false);
                            sidebar_error.set(Some(create_error.into_message()));
                        }
                    }
                });
            }
        >
            <span class="session-sidebar__new-link-icon" aria-hidden="true">
                {move || session_sidebar_new_chat_icon(creating.get())}
            </span>
            <span class="session-sidebar__new-link-label">
                {move || session_sidebar_new_chat_label(creating.get())}
            </span>
        </button>
    }
    .into_any()
}

#[cfg(not(target_family = "wasm"))]
fn session_sidebar_new_chat_button(current_workspace_id: Signal<Option<String>>) -> AnyView {
    let can_create =
        session_sidebar_new_chat_workspace_id(current_workspace_id.get_untracked()).is_some();

    view! {
        <button type="button" class="session-sidebar__new-link" prop:disabled=!can_create>
            <span class="session-sidebar__new-link-icon" aria-hidden="true">
                {session_sidebar_new_chat_icon(false)}
            </span>
            <span class="session-sidebar__new-link-label">
                {session_sidebar_new_chat_label(false)}
            </span>
        </button>
    }
    .into_any()
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
            let sidebar_error = RwSignal::new(None::<String>);
            let sidebar_open = RwSignal::new(true);
            let current_workspace_id = Signal::derive(|| Some("workspace-1".to_string()));
            let _ = view! {
                <SessionSidebarHeader
                    current_session_id="session-1".to_string()
                    current_workspace_id=current_workspace_id
                    auth_error=auth_error
                    sidebar_error=sidebar_error
                    sidebar_open=sidebar_open
                />
            };
        });
    }

    #[test]
    fn session_sidebar_new_chat_helpers_cover_ready_and_busy_states() {
        assert_eq!(
            session_sidebar_new_chat_workspace_id(Some("workspace-1".to_string())),
            Some("workspace-1".to_string())
        );
        assert_eq!(session_sidebar_new_chat_workspace_id(Some("   ".to_string())), None);
        assert_eq!(session_sidebar_new_chat_workspace_id(None), None);
        assert_eq!(session_sidebar_new_chat_icon(false), "+");
        assert_eq!(session_sidebar_new_chat_icon(true), "…");
        assert_eq!(session_sidebar_new_chat_label(false), "New chat");
        assert_eq!(session_sidebar_new_chat_label(true), "Creating…");
        assert_eq!(session_sidebar_workspaces_icon(), "▦");
        assert_eq!(session_sidebar_workspaces_label(), "Workspaces");
        assert_eq!(
            session_sidebar_workspaces_href("session-1"),
            "/app/workspaces/?return_to=%2Fapp%2Fsessions%2Fsession-1"
        );
    }
}
