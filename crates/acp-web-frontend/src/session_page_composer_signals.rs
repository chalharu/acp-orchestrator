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
        slash_palette_visible: Signal::derive(move || slash_palette_is_visible(&signals.draft.get())),
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
