use leptos::prelude::*;

use crate::{
    session_page_shell_signals::SessionShellSignals,
    session_page_sidebar_header::SessionSidebarHeader, session_page_sidebar_nav::SessionSidebarNav,
    session_page_sidebar_status::SessionSidebarStatus,
    session_page_sidebar_styles::session_sidebar_class,
};

#[derive(Clone, Copy)]
pub(super) struct SessionSidebarListControls {
    pub(super) renaming_session_id: RwSignal<Option<String>>,
    pub(super) saving_rename_session_id: RwSignal<Option<String>>,
    pub(super) rename_draft: RwSignal<String>,
    pub(super) on_rename_session: Callback<(String, String)>,
    pub(super) on_delete_session: Callback<String>,
}

#[component]
fn SessionSidebarWorkspace(#[prop(into)] current_workspace: Signal<Option<String>>) -> impl IntoView {
    move || {
        current_workspace.get().map(|workspace| {
            view! {
                <p class="session-sidebar__workspace muted" aria-label="Current workspace">
                    "Workspace: "
                    <strong>{workspace}</strong>
                </p>
            }
        })
    }
}

#[component]
pub(super) fn SessionSidebar(
    current_session_id: String,
    auth_error: RwSignal<Option<String>>,
    shell_signals: SessionShellSignals,
    sidebar_open: RwSignal<bool>,
    list_controls: SessionSidebarListControls,
) -> impl IntoView {
    let current_session_id_for_nav = current_session_id.clone();
    let has_session_items = Signal::derive(move || !shell_signals.sessions.get().is_empty());

    view! {
        <aside class=move || session_sidebar_class(sidebar_open.get())>
            <SessionSidebarHeader
                current_session_id=current_session_id
                auth_error=auth_error
                sidebar_open=sidebar_open
            />
            <SessionSidebarWorkspace current_workspace=shell_signals.current_workspace />
            <SessionSidebarStatus
                session_list_error=shell_signals.list.error
                has_session_items=has_session_items
            />
            <SessionSidebarNav
                current_session_id=current_session_id_for_nav
                sessions=shell_signals.sessions
                session_list_loaded=shell_signals.list.loaded
                session_list_error=shell_signals.list.error
                deleting_session_id=shell_signals.list.deleting_id
                delete_disabled=shell_signals.delete_disabled
                renaming_session_id=list_controls.renaming_session_id
                saving_rename_session_id=list_controls.saving_rename_session_id
                rename_draft=list_controls.rename_draft
                on_rename_session=list_controls.on_rename_session
                on_delete_session=list_controls.on_delete_session
            />
        </aside>
    }
}
