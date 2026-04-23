use leptos::prelude::*;

use crate::session_lifecycle::{SessionLifecycle, TurnState};
use crate::session_page_signals::SessionSignals;
use crate::session_state::{
    session_composer_cancel_visible, session_composer_disabled, session_composer_status_message,
};
use crate::slash::{slash_palette_is_visible, slash_palette_should_apply_selected};

#[derive(Clone, Copy)]
pub(crate) struct SessionComposerSignals {
    pub(crate) disabled: Signal<bool>,
    pub(crate) status: Signal<String>,
    pub(crate) cancel_visible: Signal<bool>,
    pub(crate) cancel_busy: Signal<bool>,
    pub(crate) slash_palette_visible: Signal<bool>,
    pub(crate) slash_candidates: Signal<Vec<acp_contracts_slash::CompletionCandidate>>,
    pub(crate) slash_selected_index: Signal<usize>,
    pub(crate) slash_apply_selected: Signal<bool>,
}

pub(crate) fn session_composer_signals(
    signals: SessionSignals,
    current_session_deleting: Signal<bool>,
) -> SessionComposerSignals {
    SessionComposerSignals {
        disabled: session_composer_disabled_signal(
            signals.turn_state,
            signals.session_status,
            current_session_deleting,
        ),
        status: session_composer_status_signal(
            signals.turn_state,
            signals.session_status,
            current_session_deleting,
        ),
        cancel_visible: session_composer_cancel_visible_signal(
            signals.turn_state,
            signals.pending_permissions,
            current_session_deleting,
        ),
        cancel_busy: session_composer_cancel_busy_signal(
            signals.turn_state,
            signals.pending_action_busy,
            current_session_deleting,
        ),
        slash_palette_visible: Signal::derive(move || {
            slash_palette_is_visible(&signals.draft.get())
        }),
        slash_candidates: Signal::derive(move || signals.slash.candidates.get()),
        slash_selected_index: Signal::derive(move || signals.slash.selected_index.get()),
        slash_apply_selected: Signal::derive(move || {
            slash_palette_should_apply_selected(
                &signals.draft.get(),
                &signals.slash.candidates.get(),
                signals.slash.selected_index.get(),
            )
        }),
    }
}

fn session_composer_disabled_signal(
    turn_state: RwSignal<TurnState>,
    session_status: RwSignal<SessionLifecycle>,
    current_session_deleting: Signal<bool>,
) -> Signal<bool> {
    Signal::derive(move || {
        session_composer_disabled(
            session_status.get(),
            turn_state.get(),
            current_session_deleting.get(),
        )
    })
}

fn session_composer_status_signal(
    turn_state: RwSignal<TurnState>,
    session_status: RwSignal<SessionLifecycle>,
    current_session_deleting: Signal<bool>,
) -> Signal<String> {
    Signal::derive(move || {
        session_composer_status_message(
            session_status.get(),
            turn_state.get(),
            current_session_deleting.get(),
        )
    })
}

fn session_composer_cancel_visible_signal(
    turn_state: RwSignal<TurnState>,
    pending_permissions: RwSignal<Vec<acp_contracts_permissions::PermissionRequest>>,
    current_session_deleting: Signal<bool>,
) -> Signal<bool> {
    Signal::derive(move || {
        session_composer_cancel_visible(
            turn_state.get(),
            !pending_permissions.get().is_empty(),
            current_session_deleting.get(),
        )
    })
}

fn session_composer_cancel_busy_signal(
    turn_state: RwSignal<TurnState>,
    pending_action_busy: RwSignal<bool>,
    current_session_deleting: Signal<bool>,
) -> Signal<bool> {
    Signal::derive(move || {
        pending_action_busy.get()
            || current_session_deleting.get()
            || matches!(turn_state.get(), TurnState::Cancelling)
    })
}

#[cfg(test)]
mod tests {
    use acp_contracts_permissions::PermissionRequest;
    use acp_contracts_slash::{CompletionCandidate, CompletionKind};
    use leptos::prelude::*;

    use super::*;
    use crate::session_page_signals::session_signals;

    fn permission(id: &str) -> PermissionRequest {
        PermissionRequest {
            request_id: id.to_string(),
            summary: format!("summary for {id}"),
        }
    }

    fn candidate(label: &str) -> CompletionCandidate {
        CompletionCandidate {
            label: label.to_string(),
            insert_text: label.to_string(),
            detail: "detail".to_string(),
            kind: CompletionKind::Command,
        }
    }

    #[test]
    fn session_composer_signals_derive_runtime_state() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            let current_session_deleting = RwSignal::new(false);
            let composer_signals = session_composer_signals(
                signals,
                Signal::derive(move || current_session_deleting.get()),
            );

            signals.session_status.set(SessionLifecycle::Active);
            signals.turn_state.set(TurnState::Idle);
            signals.draft.set("/he".to_string());
            signals.slash.candidates.set(vec![candidate("/help")]);
            signals.slash.selected_index.set(0);

            assert!(!composer_signals.disabled.get());
            assert_eq!(composer_signals.status.get(), "");
            assert!(!composer_signals.cancel_visible.get());
            assert!(!composer_signals.cancel_busy.get());
            assert!(composer_signals.slash_palette_visible.get());
            assert_eq!(composer_signals.slash_candidates.get().len(), 1);
            assert_eq!(composer_signals.slash_selected_index.get(), 0);
            assert!(composer_signals.slash_apply_selected.get());

            signals.turn_state.set(TurnState::AwaitingReply);
            assert_eq!(composer_signals.status.get(), "Waiting for response...");
            assert!(composer_signals.cancel_visible.get());

            signals.turn_state.set(TurnState::Cancelling);
            assert!(composer_signals.cancel_busy.get());

            signals.turn_state.set(TurnState::AwaitingPermission);
            signals.pending_permissions.set(vec![permission("req-1")]);
            assert_eq!(
                composer_signals.status.get(),
                "Resolve the pending request before sending another message."
            );
            assert!(!composer_signals.cancel_visible.get());

            signals.pending_action_busy.set(true);
            assert!(composer_signals.cancel_busy.get());

            signals.pending_action_busy.set(false);
            current_session_deleting.set(true);
            assert!(composer_signals.disabled.get());
            assert_eq!(composer_signals.status.get(), "Deleting session...");
            assert!(composer_signals.cancel_busy.get());
        });
    }
}
