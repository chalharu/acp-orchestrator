use acp_contracts_sessions::{SessionListItem, SessionStatus};
use leptos::prelude::*;

use crate::routing::app_session_path;
use crate::session_page_sidebar_item::SessionSidebarItem;

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

#[derive(Clone, Copy)]
struct SessionSidebarItemRuntime {
    deleting_session_id: Signal<Option<String>>,
    delete_disabled: Signal<bool>,
    renaming_session_id: RwSignal<Option<String>>,
    saving_rename_session_id: Signal<Option<String>>,
    rename_draft: RwSignal<String>,
    on_rename_session: Callback<(String, String)>,
    on_delete_session: Callback<String>,
}

#[component]
pub(super) fn SessionSidebarList(
    current_session_id: String,
    #[prop(into)] sessions: Signal<Vec<SessionListItem>>,
    #[prop(into)] deleting_session_id: Signal<Option<String>>,
    #[prop(into)] delete_disabled: Signal<bool>,
    renaming_session_id: RwSignal<Option<String>>,
    #[prop(into)] saving_rename_session_id: Signal<Option<String>>,
    rename_draft: RwSignal<String>,
    on_rename_session: Callback<(String, String)>,
    on_delete_session: Callback<String>,
) -> impl IntoView {
    session_sidebar_list_view(SessionSidebarListArgs {
        current_session_id,
        sessions,
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
fn session_sidebar_list_view(args: SessionSidebarListArgs) -> impl IntoView {
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
    let item_runtime = SessionSidebarItemRuntime {
        deleting_session_id,
        delete_disabled,
        renaming_session_id,
        saving_rename_session_id,
        rename_draft,
        on_rename_session,
        on_delete_session,
    };
    let current_session_id_for_items = current_session_id.clone();

    view! {
        <ul class="session-sidebar__list">
            <For
                each=move || sessions.get()
                key=|item| item.id.clone()
                children=move |item| {
                    session_sidebar_list_item_view(
                        item,
                        current_session_id_for_items.clone(),
                        item_runtime,
                    )
                }
            />
        </ul>
    }
}

#[cfg(not(target_family = "wasm"))]
fn session_sidebar_list_view(args: SessionSidebarListArgs) -> impl IntoView {
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
    let items = sessions.get_untracked();
    let item_runtime = SessionSidebarItemRuntime {
        deleting_session_id,
        delete_disabled,
        renaming_session_id,
        saving_rename_session_id,
        rename_draft,
        on_rename_session,
        on_delete_session,
    };

    view! {
        <ul class="session-sidebar__list">
            {items
                .into_iter()
                .map(|item| {
                    session_sidebar_list_item_view(item, current_session_id.clone(), item_runtime)
                })
                .collect_view()}
        </ul>
    }
}

fn session_sidebar_list_item_view(
    item: SessionListItem,
    current_session_id: String,
    runtime: SessionSidebarItemRuntime,
) -> impl IntoView {
    let id = item.id.clone();
    let href = app_session_path(&item.id);
    let title = session_sidebar_title(item.title);
    let activity_label = format!("Updated {}", item.last_activity_at.format("%Y-%m-%d %H:%M UTC"));
    let is_current = id == current_session_id;
    let is_closed = matches!(item.status, SessionStatus::Closed);

    view! {
        <SessionSidebarItem
            id=id
            href=href
            title=title
            activity_label=activity_label
            is_current=is_current
            is_closed=is_closed
            deleting_session_id=runtime.deleting_session_id
            delete_disabled=runtime.delete_disabled
            renaming_session_id=runtime.renaming_session_id
            saving_rename_session_id=runtime.saving_rename_session_id
            rename_draft=runtime.rename_draft
            on_rename_session=runtime.on_rename_session
            on_delete_session=runtime.on_delete_session
        />
    }
}

fn session_sidebar_title(title: String) -> String {
    if title.is_empty() {
        "New chat".to_string()
    } else {
        title
    }
}

#[cfg(test)]
mod tests {
    use acp_contracts_sessions::{SessionListItem, SessionStatus};
    use chrono::{TimeZone, Utc};
    use leptos::prelude::*;

    use super::SessionSidebarList;

    fn sample_sidebar_session() -> SessionListItem {
        SessionListItem {
            id: "s1".to_string(),
            title: "Test session".to_string(),
            status: SessionStatus::Active,
            last_activity_at: Utc.with_ymd_and_hms(2026, 4, 17, 1, 0, 0).unwrap(),
        }
    }

    #[test]
    fn session_sidebar_list_and_item_build_without_panicking() {
        let owner = Owner::new();
        owner.with(|| {
            let sessions = Signal::derive(|| vec![sample_sidebar_session()]);
            let deleting_session_id = Signal::derive(|| None::<String>);
            let saving_rename_session_id = Signal::derive(|| None::<String>);
            let rename_draft = RwSignal::new(String::new());
            let renaming_session_id = RwSignal::new(None::<String>);

            let _ = view! {
                <SessionSidebarList
                    current_session_id="s1".to_string()
                    sessions=sessions
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
}
