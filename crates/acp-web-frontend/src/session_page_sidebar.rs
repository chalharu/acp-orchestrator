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
            <SessionSidebarStatus
                workspace_message=shell_signals.current_workspace
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

#[cfg(test)]
mod tests {
    use acp_contracts_sessions::{SessionListItem, SessionStatus};
    use chrono::{TimeZone, Utc};
    use leptos::prelude::*;

    use super::{SessionSidebar, SessionSidebarListControls};
    use crate::session_page_shell_signals::session_shell_signals;
    use crate::session_page_signals::session_signals;

    fn sample_sidebar_controls() -> SessionSidebarListControls {
        SessionSidebarListControls {
            renaming_session_id: RwSignal::new(None::<String>),
            saving_rename_session_id: RwSignal::new(None::<String>),
            rename_draft: RwSignal::new(String::new()),
            on_rename_session: Callback::new(|_| {}),
            on_delete_session: Callback::new(|_| {}),
        }
    }

    fn sample_session_list_item() -> SessionListItem {
        SessionListItem {
            id: "session-1".to_string(),
            workspace_id: "workspace-1".to_string(),
            title: "Session 1".to_string(),
            status: SessionStatus::Active,
            last_activity_at: Utc.with_ymd_and_hms(2026, 4, 23, 19, 0, 0).unwrap(),
        }
    }

    #[test]
    fn session_sidebar_builds_with_workspace_aware_shell_signals() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals
                .current_workspace_name
                .set(Some("Workspace A".to_string()));
            signals.list.loaded.set(true);
            signals.list.items.set(vec![sample_session_list_item()]);

            let shell_signals = session_shell_signals(signals);
            let _ = view! {
                <SessionSidebar
                    current_session_id="session-1".to_string()
                    auth_error=signals.action_error
                    shell_signals=shell_signals
                    sidebar_open=RwSignal::new(true)
                    list_controls=sample_sidebar_controls()
                />
            };
        });
    }
}
