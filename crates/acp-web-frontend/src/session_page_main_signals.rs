use leptos::prelude::*;

use crate::session_lifecycle::{BadgeTone, SessionLifecycle, TurnState};
use crate::session_page_entries::SessionEntry;
use crate::session_page_signals::SessionSignals;

#[derive(Clone, Copy)]
pub(crate) struct SessionMainSignals {
    pub(crate) session_status: Signal<SessionLifecycle>,
    pub(crate) topbar_message: Signal<Option<String>>,
    pub(crate) connection_badge: Signal<(&'static str, &'static str, BadgeTone)>,
    pub(crate) worker_badge: Signal<(&'static str, &'static str, BadgeTone)>,
    pub(crate) entries: Signal<Vec<SessionEntry>>,
    pub(crate) pending_permissions: Signal<Vec<acp_contracts_permissions::PermissionRequest>>,
    pub(crate) pending_action_busy: Signal<bool>,
}

pub(crate) fn session_main_signals(signals: SessionSignals) -> SessionMainSignals {
    let entries = signals.entries;
    let pending_action_busy = signals.pending_action_busy;
    let action_error = signals.action_error;
    let connection_error = signals.connection_error;
    let pending_permissions = signals.pending_permissions;
    let session_status = signals.session_status;
    let turn_state = signals.turn_state;

    SessionMainSignals {
        session_status: Signal::derive(move || session_status.get()),
        topbar_message: Signal::derive(move || action_error.get().or(connection_error.get())),
        connection_badge: Signal::derive(move || {
            main_connection_badge(session_status.get(), connection_error.get().is_some())
        }),
        worker_badge: Signal::derive(move || {
            main_worker_badge(
                session_status.get(),
                turn_state.get(),
                !pending_permissions.get().is_empty(),
            )
        }),
        entries: Signal::derive(move || entries.get()),
        pending_permissions: Signal::derive(move || pending_permissions.get()),
        pending_action_busy: Signal::derive(move || pending_action_busy.get()),
    }
}

fn main_connection_badge(
    session_status: SessionLifecycle,
    has_connection_error: bool,
) -> (&'static str, &'static str, BadgeTone) {
    match session_status {
        SessionLifecycle::Loading => ("Connection", "connecting", BadgeTone::Neutral),
        SessionLifecycle::Active if has_connection_error => {
            ("Connection", "reconnecting", BadgeTone::Warning)
        }
        SessionLifecycle::Active => ("Connection", "live", BadgeTone::Success),
        SessionLifecycle::Closed => ("Connection", "ended", BadgeTone::Neutral),
        SessionLifecycle::Unavailable | SessionLifecycle::Error => {
            ("Connection", "unavailable", BadgeTone::Danger)
        }
    }
}

fn main_worker_badge(
    session_status: SessionLifecycle,
    turn_state: TurnState,
    has_pending_permissions: bool,
) -> (&'static str, &'static str, BadgeTone) {
    match session_status {
        SessionLifecycle::Loading => ("Worker", "starting", BadgeTone::Neutral),
        SessionLifecycle::Unavailable | SessionLifecycle::Error => {
            ("Worker", "unavailable", BadgeTone::Danger)
        }
        SessionLifecycle::Closed => ("Worker", "stopped", BadgeTone::Neutral),
        SessionLifecycle::Active if has_pending_permissions => {
            ("Worker", "permission", BadgeTone::Warning)
        }
        SessionLifecycle::Active => match turn_state {
            TurnState::Submitting | TurnState::AwaitingReply => {
                ("Worker", "running", BadgeTone::Success)
            }
            TurnState::Cancelling => ("Worker", "cancelling", BadgeTone::Warning),
            TurnState::AwaitingPermission => ("Worker", "permission", BadgeTone::Warning),
            TurnState::Idle => ("Worker", "idle", BadgeTone::Neutral),
        },
    }
}

#[cfg(test)]
pub mod tests {
    use super::session_main_signals;
    use crate::session_lifecycle::BadgeTone;
    use crate::session_page_signals::session_signals;
    use leptos::prelude::*;

    pub fn badge(
        label: &'static str,
        value: &'static str,
        tone: BadgeTone,
    ) -> (&'static str, &'static str, BadgeTone) {
        (label, value, tone)
    }

    #[test]
    fn badge_helper_builds_status_badge() {
        let badge = badge("Connection", "live", BadgeTone::Success);
        assert_eq!(badge.0, "Connection");
        assert_eq!(badge.1, "live");
        assert_eq!(badge.2, BadgeTone::Success);
    }

    #[test]
    fn session_main_signals_derives_from_session_signals() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            let main_signals = session_main_signals(signals);
            // Just verify it compiles and runs without panic
            let _ = main_signals.entries.get();
        });
    }
}
