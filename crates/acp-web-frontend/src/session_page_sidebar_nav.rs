use acp_contracts_sessions::SessionListItem;
use leptos::prelude::*;

use crate::{
    session_page_sidebar_list::SessionSidebarList,
    session_page_sidebar_styles::session_sidebar_empty_message,
};

struct SessionSidebarNavArgs {
    current_session_id: String,
    sessions: Signal<Vec<SessionListItem>>,
    session_list_loaded: Signal<bool>,
    session_list_error: Signal<Option<String>>,
    deleting_session_id: Signal<Option<String>>,
    delete_disabled: Signal<bool>,
    renaming_session_id: RwSignal<Option<String>>,
    saving_rename_session_id: Signal<Option<String>>,
    rename_draft: RwSignal<String>,
    on_rename_session: Callback<(String, String)>,
    on_delete_session: Callback<String>,
}

#[component]
pub(super) fn SessionSidebarNav(
    current_session_id: String,
    #[prop(into)] sessions: Signal<Vec<SessionListItem>>,
    #[prop(into)] session_list_loaded: Signal<bool>,
    #[prop(into)] session_list_error: Signal<Option<String>>,
    #[prop(into)] deleting_session_id: Signal<Option<String>>,
    #[prop(into)] delete_disabled: Signal<bool>,
    renaming_session_id: RwSignal<Option<String>>,
    #[prop(into)] saving_rename_session_id: Signal<Option<String>>,
    rename_draft: RwSignal<String>,
    on_rename_session: Callback<(String, String)>,
    on_delete_session: Callback<String>,
) -> impl IntoView {
    session_sidebar_nav_view(SessionSidebarNavArgs {
        current_session_id,
        sessions,
        session_list_loaded,
        session_list_error,
        deleting_session_id,
        delete_disabled,
        renaming_session_id,
        saving_rename_session_id,
        rename_draft,
        on_rename_session,
        on_delete_session,
    })
}

#[cfg(target_family = "wasm")]
fn session_sidebar_nav_view(args: SessionSidebarNavArgs) -> impl IntoView {
    let SessionSidebarNavArgs {
        current_session_id,
        sessions,
        session_list_loaded,
        session_list_error,
        deleting_session_id,
        delete_disabled,
        renaming_session_id,
        saving_rename_session_id,
        rename_draft,
        on_rename_session,
        on_delete_session,
    } = args;

    view! {
        <nav class="session-sidebar__nav" aria-label="Sessions">
            <Show
                when=move || session_list_loaded.get()
                fallback=|| {
                    view! { <p class="session-sidebar__empty muted">"Loading sessions..."</p> }
                }
            >
                <Show
                    when=move || !sessions.get().is_empty()
                    fallback=move || {
                        view! {
                            <p class="session-sidebar__empty muted">
                                {move || session_sidebar_empty_message(session_list_error.get().is_some())}
                            </p>
                        }
                    }
                >
                    <SessionSidebarList
                        current_session_id=current_session_id.clone()
                        sessions=sessions
                        deleting_session_id=deleting_session_id
                        delete_disabled=delete_disabled
                        renaming_session_id=renaming_session_id
                        saving_rename_session_id=saving_rename_session_id
                        rename_draft=rename_draft
                        on_rename_session=on_rename_session
                        on_delete_session=on_delete_session
                    />
                </Show>
            </Show>
        </nav>
    }
}

#[cfg(not(target_family = "wasm"))]
fn session_sidebar_nav_view(args: SessionSidebarNavArgs) -> impl IntoView {
    let SessionSidebarNavArgs {
        current_session_id,
        sessions,
        session_list_loaded,
        session_list_error,
        deleting_session_id,
        delete_disabled,
        renaming_session_id,
        saving_rename_session_id,
        rename_draft,
        on_rename_session,
        on_delete_session,
    } = args;
    let loaded = session_list_loaded.get_untracked();
    let items = sessions.get_untracked();
    let has_error = session_list_error.get_untracked().is_some();

    if !loaded {
        return session_sidebar_loading_view().into_any();
    }

    if items.is_empty() {
        return session_sidebar_empty_view(has_error).into_any();
    }

    session_sidebar_loaded_view(SessionSidebarListArgs {
        current_session_id,
        sessions: Signal::derive(move || items.clone()),
        deleting_session_id,
        delete_disabled,
        renaming_session_id,
        saving_rename_session_id,
        rename_draft,
        on_rename_session,
        on_delete_session,
    })
    .into_any()
}

#[cfg(not(target_family = "wasm"))]
struct SessionSidebarListArgs {
    current_session_id: String,
    sessions: Signal<Vec<SessionListItem>>,
    deleting_session_id: Signal<Option<String>>,
    delete_disabled: Signal<bool>,
    renaming_session_id: RwSignal<Option<String>>,
    saving_rename_session_id: Signal<Option<String>>,
    rename_draft: RwSignal<String>,
    on_rename_session: Callback<(String, String)>,
    on_delete_session: Callback<String>,
}

#[cfg(not(target_family = "wasm"))]
fn session_sidebar_loading_view() -> impl IntoView {
    view! {
        <nav class="session-sidebar__nav" aria-label="Sessions">
            <p class="session-sidebar__empty muted">"Loading sessions..."</p>
        </nav>
    }
}

#[cfg(not(target_family = "wasm"))]
fn session_sidebar_empty_view(has_error: bool) -> impl IntoView {
    view! {
        <nav class="session-sidebar__nav" aria-label="Sessions">
            <p class="session-sidebar__empty muted">{session_sidebar_empty_message(has_error)}</p>
        </nav>
    }
}

#[cfg(not(target_family = "wasm"))]
fn session_sidebar_loaded_view(args: SessionSidebarListArgs) -> impl IntoView {
    let SessionSidebarListArgs {
        current_session_id,
        sessions,
        deleting_session_id,
        delete_disabled,
        renaming_session_id,
        saving_rename_session_id,
        rename_draft,
        on_rename_session,
        on_delete_session,
    } = args;

    view! {
        <nav class="session-sidebar__nav" aria-label="Sessions">
            <SessionSidebarList
                current_session_id=current_session_id
                sessions=sessions
                deleting_session_id=deleting_session_id
                delete_disabled=delete_disabled
                renaming_session_id=renaming_session_id
                saving_rename_session_id=saving_rename_session_id
                rename_draft=rename_draft
                on_rename_session=on_rename_session
                on_delete_session=on_delete_session
            />
        </nav>
    }
}

#[cfg(test)]
mod tests {
    use acp_contracts_sessions::{SessionListItem, SessionStatus};
    use chrono::{TimeZone, Utc};
    use leptos::prelude::*;

    use super::SessionSidebarNav;

    fn sample_sidebar_session() -> SessionListItem {
        SessionListItem {
            id: "s1".to_string(),
            title: "Test session".to_string(),
            status: SessionStatus::Active,
            last_activity_at: Utc.with_ymd_and_hms(2026, 4, 17, 1, 0, 0).unwrap(),
        }
    }

    #[test]
    fn sidebar_nav_component_builds_without_panicking() {
        let owner = Owner::new();
        owner.with(|| {
            let sessions = Signal::derive(|| vec![sample_sidebar_session()]);
            let deleting_session_id = Signal::derive(|| None::<String>);
            let saving_rename_session_id = Signal::derive(|| None::<String>);
            let rename_draft = RwSignal::new("Draft".to_string());
            let renaming_session_id = RwSignal::new(None::<String>);

            let _ = view! {
                <SessionSidebarNav
                    current_session_id="s1".to_string()
                    sessions=sessions
                    session_list_loaded=Signal::derive(|| true)
                    session_list_error=Signal::derive(|| None::<String>)
                    deleting_session_id=deleting_session_id
                    delete_disabled=Signal::derive(|| false)
                    renaming_session_id=renaming_session_id
                    saving_rename_session_id=saving_rename_session_id
                    rename_draft=rename_draft
                    on_rename_session=Callback::new(|_: (String, String)| {})
                    on_delete_session=Callback::new(|_: String| {})
                />
            };
        });
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn session_sidebar_nav_builds_empty_state_without_panicking() {
        let owner = Owner::new();
        owner.with(|| {
            let _ = view! {
                <SessionSidebarNav
                    current_session_id="s1".to_string()
                    sessions=Signal::derive(Vec::<SessionListItem>::new)
                    session_list_loaded=Signal::derive(|| true)
                    session_list_error=Signal::derive(|| Some("temporary".to_string()))
                    deleting_session_id=Signal::derive(|| None::<String>)
                    delete_disabled=Signal::derive(|| false)
                    renaming_session_id=RwSignal::new(None::<String>)
                    saving_rename_session_id=Signal::derive(|| None::<String>)
                    rename_draft=RwSignal::new(String::new())
                    on_rename_session=Callback::new(|_: (String, String)| {})
                    on_delete_session=Callback::new(|_: String| {})
                />
            };
        });
    }
}
