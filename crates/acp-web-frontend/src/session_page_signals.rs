#![cfg_attr(not(target_family = "wasm"), allow(dead_code))]

use acp_contracts_permissions::PermissionRequest;
use acp_contracts_sessions::SessionListItem;
use acp_contracts_slash::CompletionCandidate;
use futures_util::future::AbortHandle;
use leptos::prelude::*;
use web_sys::EventSource;

use crate::{
    browser::{load_draft, save_draft},
    session_page_entries::SessionEntry,
    session_lifecycle::{SessionLifecycle, TurnState},
};

#[derive(Clone, Copy)]
pub(super) struct SlashSignals {
    pub(super) candidates: RwSignal<Vec<CompletionCandidate>>,
    pub(super) selected_index: RwSignal<usize>,
}

#[derive(Clone, Copy)]
pub(super) struct SessionListSignals {
    pub(super) items: RwSignal<Vec<SessionListItem>>,
    pub(super) loaded: RwSignal<bool>,
    pub(super) error: RwSignal<Option<String>>,
    pub(super) deleting_id: RwSignal<Option<String>>,
    pub(super) renaming_id: RwSignal<Option<String>>,
    pub(super) saving_rename_id: RwSignal<Option<String>>,
    pub(super) rename_draft: RwSignal<String>,
}

#[derive(Clone, Copy)]
pub(super) struct SessionSignals {
    pub(super) entries: RwSignal<Vec<SessionEntry>>,
    pub(super) pending_permissions: RwSignal<Vec<PermissionRequest>>,
    pub(super) action_error: RwSignal<Option<String>>,
    pub(super) connection_error: RwSignal<Option<String>>,
    pub(super) event_source: RwSignal<Option<EventSource>>,
    pub(super) stream_abort: RwSignal<Option<AbortHandle>>,
    pub(super) session_status: RwSignal<SessionLifecycle>,
    pub(super) turn_state: RwSignal<TurnState>,
    pub(super) pending_action_busy: RwSignal<bool>,
    pub(super) draft: RwSignal<String>,
    pub(super) slash: SlashSignals,
    pub(super) list: SessionListSignals,
    pub(super) tool_activity_serial: RwSignal<u64>,
}

pub(super) fn current_session_deleting_signal(
    session_id: String,
    signals: SessionSignals,
) -> Signal<bool> {
    Signal::derive(move || signals.list.deleting_id.get().as_deref() == Some(session_id.as_str()))
}

pub(super) fn restore_session_draft(session_id: &str, signals: SessionSignals) {
    apply_restored_session_draft(signals.draft, load_draft(session_id));
}

pub(super) fn persist_session_draft(session_id: String, draft: RwSignal<String>) {
    #[cfg(target_family = "wasm")]
    Effect::new(move |_| {
        persist_session_draft_text(&session_id, &draft.get());
    });

    #[cfg(not(target_family = "wasm"))]
    persist_session_draft_text(&session_id, &draft.get_untracked());
}

fn apply_restored_session_draft(draft: RwSignal<String>, stored_draft: String) {
    if !stored_draft.is_empty() {
        draft.set(stored_draft);
    }
}

fn persist_session_draft_text(session_id: &str, text: &str) {
    save_draft(session_id, text);
}

pub(super) fn session_signals() -> SessionSignals {
    SessionSignals {
        entries: RwSignal::new(Vec::new()),
        pending_permissions: RwSignal::new(Vec::new()),
        action_error: RwSignal::new(None::<String>),
        connection_error: RwSignal::new(None::<String>),
        event_source: RwSignal::new(None::<EventSource>),
        stream_abort: RwSignal::new(None::<AbortHandle>),
        session_status: RwSignal::new(SessionLifecycle::Loading),
        turn_state: RwSignal::new(TurnState::Idle),
        pending_action_busy: RwSignal::new(false),
        draft: RwSignal::new(String::new()),
        slash: SlashSignals {
            candidates: RwSignal::new(Vec::new()),
            selected_index: RwSignal::new(0),
        },
        list: SessionListSignals {
            items: RwSignal::new(Vec::new()),
            loaded: RwSignal::new(false),
            error: RwSignal::new(None::<String>),
            deleting_id: RwSignal::new(None::<String>),
            renaming_id: RwSignal::new(None::<String>),
            saving_rename_id: RwSignal::new(None::<String>),
            rename_draft: RwSignal::new(String::new()),
        },
        tool_activity_serial: RwSignal::new(0),
    }
}

#[cfg(test)]
mod tests {
    use leptos::prelude::*;

    use super::apply_restored_session_draft;

    #[test]
    fn apply_restored_session_draft_sets_non_empty_text() {
        let owner = Owner::new();
        owner.with(|| {
            let draft = RwSignal::new(String::new());

            apply_restored_session_draft(draft, "saved draft".to_string());

            assert_eq!(draft.get_untracked(), "saved draft");
        });
    }

    #[test]
    fn apply_restored_session_draft_keeps_existing_text_for_empty_restore() {
        let owner = Owner::new();
        owner.with(|| {
            let draft = RwSignal::new("current".to_string());

            apply_restored_session_draft(draft, String::new());

            assert_eq!(draft.get_untracked(), "current");
        });
    }
}
