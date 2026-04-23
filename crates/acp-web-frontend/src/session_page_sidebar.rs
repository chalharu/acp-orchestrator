use acp_contracts_sessions::SessionListItem;
use leptos::prelude::*;

use crate::{
    session_page_sidebar_header::SessionSidebarHeader,
    session_page_sidebar_nav::SessionSidebarNav,
    session_page_sidebar_status::SessionSidebarStatus,
    session_page_sidebar_styles::session_sidebar_class,
};

#[component]
pub(super) fn SessionSidebar(
    current_session_id: String,
    auth_error: RwSignal<Option<String>>,
    #[prop(into)] sessions: Signal<Vec<SessionListItem>>,
    #[prop(into)] session_list_loaded: Signal<bool>,
    #[prop(into)] session_list_error: Signal<Option<String>>,
    sidebar_open: RwSignal<bool>,
    #[prop(into)] deleting_session_id: Signal<Option<String>>,
    #[prop(into)] delete_disabled: Signal<bool>,
    renaming_session_id: RwSignal<Option<String>>,
    #[prop(into)] saving_rename_session_id: Signal<Option<String>>,
    rename_draft: RwSignal<String>,
    on_rename_session: Callback<(String, String)>,
    on_delete_session: Callback<String>,
) -> impl IntoView {
    let current_session_id_for_nav = current_session_id.clone();
    let has_session_items = Signal::derive(move || !sessions.get().is_empty());

    view! {
        <aside class=move || session_sidebar_class(sidebar_open.get())>
            <SessionSidebarHeader
                current_session_id=current_session_id
                auth_error=auth_error
                sidebar_open=sidebar_open
            />
            <SessionSidebarStatus
                session_list_error=session_list_error
                has_session_items=has_session_items
            />
            <SessionSidebarNav
                current_session_id=current_session_id_for_nav
                sessions=sessions
                session_list_loaded=session_list_loaded
                session_list_error=session_list_error
                deleting_session_id=deleting_session_id
                delete_disabled=delete_disabled
                renaming_session_id=renaming_session_id
                saving_rename_session_id=saving_rename_session_id
                rename_draft=rename_draft
                on_rename_session=on_rename_session
                on_delete_session=on_delete_session
            />
        </aside>
    }
}
