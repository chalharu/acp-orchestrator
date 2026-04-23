use leptos::prelude::*;

use crate::session_page_callbacks::SessionViewCallbacks;
use crate::session_page_composer_signals::SessionComposerSignals;
use crate::session_page_dock::SessionDock;
use crate::session_page_main_signals::SessionMainSignals;
use crate::session_page_topbar::SessionTopBar;
use crate::session_page_transcript::SessionTranscriptPanel;

#[component]
pub(crate) fn SessionMain(
    main_signals: SessionMainSignals,
    sidebar_open: RwSignal<bool>,
    composer: SessionComposerSignals,
    callbacks: SessionViewCallbacks,
    draft: RwSignal<String>,
) -> impl IntoView {
    view! {
        <section class="session-main">
            <SessionTopBar
                message=main_signals.topbar_message
                connection_badge=main_signals.connection_badge
                worker_badge=main_signals.worker_badge
                sidebar_open=sidebar_open
            />
            <SessionTranscriptPanel
                entries=main_signals.entries
                session_status=main_signals.session_status
                pending_permissions=main_signals.pending_permissions
                pending_action_busy=main_signals.pending_action_busy
                on_approve=callbacks.approve
                on_deny=callbacks.deny
                on_cancel=callbacks.cancel
            />
            <SessionDock composer=composer callbacks=callbacks draft=draft />
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::SessionMain;
    use crate::session_page_callbacks::session_view_callbacks;
    use crate::session_page_composer_signals::session_composer_signals;
    use crate::session_page_main_signals::session_main_signals;
    use crate::session_page_signals::session_signals;
    use leptos::prelude::*;

    #[test]
    fn session_main_builds_without_panicking() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            let draft = signals.draft;
            let composer = session_composer_signals(signals, Signal::derive(|| false));
            let main_signals = session_main_signals(signals);
            let callbacks = session_view_callbacks("s1".to_string(), signals);
            let sidebar_open = RwSignal::new(false);

            let _ = view! {
                <SessionMain
                    main_signals=main_signals
                    sidebar_open=sidebar_open
                    composer=composer
                    callbacks=callbacks
                    draft=draft
                />
            };
        });
    }
}
