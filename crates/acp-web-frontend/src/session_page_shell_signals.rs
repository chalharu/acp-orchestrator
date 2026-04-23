use leptos::prelude::*;

use crate::session_page_signals::{SessionListSignals, SessionSignals};
use crate::session_state::session_action_busy;

#[derive(Clone, Copy)]
pub(crate) struct SessionShellSignals {
    pub(crate) sessions: Signal<Vec<acp_contracts_sessions::SessionListItem>>,
    pub(crate) list: SessionListSignals,
    pub(crate) delete_disabled: Signal<bool>,
}

pub(crate) fn session_shell_signals(signals: SessionSignals) -> SessionShellSignals {
    let session_list = signals.list.items;
    let pending_action_busy = signals.pending_action_busy;

    SessionShellSignals {
        sessions: Signal::derive(move || session_list.get()),
        list: signals.list,
        delete_disabled: Signal::derive(move || {
            session_action_busy(signals.turn_state.get(), pending_action_busy.get(), false)
        }),
    }
}
