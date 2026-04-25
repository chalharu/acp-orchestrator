use leptos::prelude::*;

use crate::session_page_signals::{SessionListSignals, SessionSignals};
use crate::session_state::session_action_busy;

#[derive(Clone, Copy)]
pub(crate) struct SessionShellSignals {
    pub(crate) sessions: Signal<Vec<acp_contracts_sessions::SessionListItem>>,
    pub(crate) list: SessionListSignals,
    pub(crate) delete_disabled: Signal<bool>,
    pub(crate) current_workspace: Signal<Option<String>>,
    pub(crate) current_workspace_id: Signal<Option<String>>,
}

fn current_workspace_label(
    workspace_name: Option<String>,
    workspace_id: Option<String>,
) -> Option<String> {
    workspace_name
        .filter(|name| !name.trim().is_empty())
        .or_else(|| workspace_id.filter(|id| !id.trim().is_empty()))
}

pub(crate) fn session_shell_signals(signals: SessionSignals) -> SessionShellSignals {
    let session_list = signals.list.items;
    let pending_action_busy = signals.pending_action_busy;
    let current_workspace_id = signals.current_workspace_id;
    let current_workspace_name = signals.current_workspace_name;

    SessionShellSignals {
        sessions: Signal::derive(move || session_list.get()),
        list: signals.list,
        delete_disabled: Signal::derive(move || {
            session_action_busy(signals.turn_state.get(), pending_action_busy.get(), false)
        }),
        current_workspace: Signal::derive(move || {
            current_workspace_label(current_workspace_name.get(), current_workspace_id.get())
        }),
        current_workspace_id: Signal::derive(move || current_workspace_id.get()),
    }
}

#[cfg(test)]
mod tests {
    use leptos::prelude::*;

    use super::*;
    use crate::session_lifecycle::TurnState;
    use crate::session_page_signals::session_signals;

    #[test]
    fn session_shell_signals_reflect_delete_disabled_state() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            let shell_signals = session_shell_signals(signals);

            assert!(!shell_signals.delete_disabled.get());

            signals.turn_state.set(TurnState::Submitting);
            assert!(shell_signals.delete_disabled.get());

            signals.turn_state.set(TurnState::Idle);
            signals.pending_action_busy.set(true);
            assert!(shell_signals.delete_disabled.get());
        });
    }

    #[test]
    fn session_shell_signals_prefer_workspace_name_and_fallback_to_id() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            let shell_signals = session_shell_signals(signals);

            assert_eq!(shell_signals.current_workspace.get(), None);

            signals
                .current_workspace_id
                .set(Some("workspace-a".to_string()));
            assert_eq!(
                shell_signals.current_workspace.get(),
                Some("workspace-a".to_string())
            );

            signals
                .current_workspace_name
                .set(Some("Workspace A".to_string()));
            assert_eq!(
                shell_signals.current_workspace.get(),
                Some("Workspace A".to_string())
            );
        });
    }
}
