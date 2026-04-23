// Re-exports for public API
pub(crate) use crate::session_page_callbacks::{SessionViewCallbacks, session_view_callbacks};
pub(crate) use crate::session_page_home::{
    SessionBackdrop, default_sidebar_open, session_layout_class,
};
pub(crate) use crate::session_page_shell_signals::{SessionShellSignals, session_shell_signals};

use leptos::prelude::*;

use crate::session_page_actions::{bind_slash_completion, spawn_session_bootstrap};
use crate::session_page_composer_signals::{SessionComposerSignals, session_composer_signals};
use crate::session_page_main::SessionMain;
use crate::session_page_main_signals::{SessionMainSignals, session_main_signals};
use crate::session_page_signals::{
    SessionSignals, current_session_deleting_signal, persist_session_draft, restore_session_draft,
    session_signals,
};
use crate::session_page_sidebar::SessionSidebar;

#[component]
pub(crate) fn SessionView(session_id: String) -> impl IntoView {
    let signals = session_signals();
    let sidebar_open = RwSignal::new(default_sidebar_open());
    let current_session_deleting = current_session_deleting_signal(session_id.clone(), signals);
    restore_session_draft(&session_id, signals);
    persist_session_draft(session_id.clone(), signals.draft);
    bind_slash_completion(signals);
    spawn_session_bootstrap(session_id.clone(), signals);

    session_view_content(
        session_id.clone(),
        signals,
        session_composer_signals(signals, current_session_deleting),
        session_view_callbacks(session_id, signals),
        sidebar_open,
    )
}

fn session_view_content(
    current_session_id: String,
    signals: SessionSignals,
    composer: SessionComposerSignals,
    callbacks: SessionViewCallbacks,
    sidebar_open: RwSignal<bool>,
) -> impl IntoView {
    let draft = signals.draft;
    let shell_signals = session_shell_signals(signals);
    let main_signals = session_main_signals(signals);

    view! {
        <SessionBackdrop sidebar_open=sidebar_open />
        <main class="app-shell app-shell--session">
            <SessionShell
                current_session_id=current_session_id
                auth_error=signals.action_error
                sidebar_open=sidebar_open
                shell_signals=shell_signals
                main_signals=main_signals
                composer=composer
                callbacks=callbacks
                draft=draft
            />
        </main>
    }
}

#[component]
pub(crate) fn SessionShell(
    current_session_id: String,
    auth_error: RwSignal<Option<String>>,
    sidebar_open: RwSignal<bool>,
    shell_signals: SessionShellSignals,
    main_signals: SessionMainSignals,
    composer: SessionComposerSignals,
    callbacks: SessionViewCallbacks,
    draft: RwSignal<String>,
) -> impl IntoView {
    let SessionViewCallbacks {
        rename_session: on_rename_session,
        delete_session: on_delete_session,
        ..
    } = callbacks;

    view! {
        <div class=move || session_layout_class(sidebar_open.get())>
            <SessionSidebar
                current_session_id=current_session_id
                auth_error=auth_error
                sessions=shell_signals.sessions
                session_list_loaded=shell_signals.list.loaded
                session_list_error=shell_signals.list.error
                sidebar_open=sidebar_open
                deleting_session_id=shell_signals.list.deleting_id
                delete_disabled=shell_signals.delete_disabled
                renaming_session_id=shell_signals.list.renaming_id
                saving_rename_session_id=shell_signals.list.saving_rename_id
                rename_draft=shell_signals.list.rename_draft
                on_rename_session=on_rename_session
                on_delete_session=on_delete_session
            />
            <SessionMain
                main_signals=main_signals
                sidebar_open=sidebar_open
                composer=composer
                callbacks=callbacks
                draft=draft
            />
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::{SessionView, session_view_callbacks};
    use crate::session_page_signals::session_signals;
    use leptos::prelude::*;

    #[test]
    fn session_view_builds_without_panicking_on_host() {
        let owner = Owner::new();
        owner.with(|| {
            let _ = view! { <SessionView session_id="session-1".to_string() /> };
        });
    }

    #[test]
    fn session_view_callbacks_build_without_panicking() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            let _ = session_view_callbacks("session-1".to_string(), signals);
        });
    }
}
