use acp_contracts_sessions::{SessionListItem, SessionStatus};
use leptos::prelude::*;

use crate::routing::app_session_path;
use crate::session_page_sidebar_item::{SessionSidebarItemProps, session_sidebar_item};

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

#[derive(Clone)]
pub(super) struct SessionSidebarListProps {
    pub(super) current_session_id: String,
    pub(super) sessions: Signal<Vec<SessionListItem>>,
    pub(super) deleting_session_id: Signal<Option<String>>,
    pub(super) delete_disabled: Signal<bool>,
    pub(super) renaming_session_id: RwSignal<Option<String>>,
    pub(super) saving_rename_session_id: Signal<Option<String>>,
    pub(super) rename_draft: RwSignal<String>,
    pub(super) on_rename_session: Callback<(String, String)>,
    pub(super) on_delete_session: Callback<String>,
}

pub(super) fn session_sidebar_list(props: SessionSidebarListProps) -> AnyView {
    let SessionSidebarListProps {
        current_session_id,
        sessions,
        deleting_session_id,
        delete_disabled,
        renaming_session_id,
        saving_rename_session_id,
        rename_draft,
        on_rename_session,
        on_delete_session,
    } = props;

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
    .into_any()
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
    let activity_label = format!(
        "Updated {}",
        item.last_activity_at.format("%Y-%m-%d %H:%M UTC")
    );
    let is_current = id == current_session_id;
    let is_closed = matches!(item.status, SessionStatus::Closed);

    session_sidebar_item(SessionSidebarItemProps {
        id,
        href,
        title,
        activity_label,
        is_current,
        is_closed,
        deleting_session_id: runtime.deleting_session_id,
        delete_disabled: runtime.delete_disabled,
        renaming_session_id: runtime.renaming_session_id,
        saving_rename_session_id: runtime.saving_rename_session_id,
        rename_draft: runtime.rename_draft,
        on_rename_session: runtime.on_rename_session,
        on_delete_session: runtime.on_delete_session,
    })
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

    use super::{SessionSidebarListProps, session_sidebar_list, session_sidebar_title};

    fn sample_sidebar_session() -> SessionListItem {
        SessionListItem {
            id: "s1".to_string(),
            workspace_id: "w_test".to_string(),
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

            let _ = session_sidebar_list(SessionSidebarListProps {
                current_session_id: "s1".to_string(),
                sessions,
                deleting_session_id,
                delete_disabled: Signal::derive(|| false),
                renaming_session_id,
                saving_rename_session_id,
                rename_draft,
                on_rename_session: Callback::new(|_: (String, String)| {}),
                on_delete_session: Callback::new(|_: String| {}),
            });
        });
    }

    #[test]
    fn session_sidebar_title_defaults_blank_titles() {
        assert_eq!(session_sidebar_title(String::new()), "New chat");
        assert_eq!(session_sidebar_title("Existing".to_string()), "Existing");
    }
}
