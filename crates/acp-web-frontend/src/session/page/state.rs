#![cfg_attr(not(target_family = "wasm"), allow(dead_code))]

use acp_contracts::{CompletionCandidate, SessionListItem};
use futures_util::future::AbortHandle;
use leptos::prelude::*;
use web_sys::EventSource;

use crate::{
    browser::{load_draft, save_draft},
    components::composer::ComposerSlashCallbacks,
    domain::session::{
        PendingPermission, SessionLifecycle, StatusBadge, TurnState, connection_badge_state,
        session_action_busy, session_composer_cancel_visible, session_composer_disabled,
        session_composer_status_message, worker_badge_state,
    },
    domain::transcript::TranscriptEntry,
    slash::{slash_palette_is_visible, slash_palette_should_apply_selected},
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
    pub(super) entries: RwSignal<Vec<TranscriptEntry>>,
    pub(super) pending_permissions: RwSignal<Vec<PendingPermission>>,
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

#[derive(Clone, Copy)]
pub(super) struct SessionViewCallbacks {
    pub(super) submit: Callback<String>,
    pub(super) approve: Callback<String>,
    pub(super) deny: Callback<String>,
    pub(super) cancel: Callback<()>,
    pub(super) slash: ComposerSlashCallbacks,
    pub(super) rename_session: Callback<(String, String)>,
    pub(super) delete_session: Callback<String>,
}

#[derive(Clone, Copy)]
pub(super) struct SessionComposerSignals {
    pub(super) disabled: Signal<bool>,
    pub(super) status: Signal<String>,
    pub(super) cancel_visible: Signal<bool>,
    pub(super) cancel_busy: Signal<bool>,
    pub(super) slash_palette_visible: Signal<bool>,
    pub(super) slash_candidates: Signal<Vec<CompletionCandidate>>,
    pub(super) slash_selected_index: Signal<usize>,
    pub(super) slash_apply_selected: Signal<bool>,
}

#[derive(Clone, Copy)]
pub(super) struct SessionShellSignals {
    pub(super) sessions: Signal<Vec<SessionListItem>>,
    pub(super) list: SessionListSignals,
    pub(super) delete_disabled: Signal<bool>,
}

#[derive(Clone, Copy)]
pub(super) struct SessionMainSignals {
    pub(super) session_status: Signal<SessionLifecycle>,
    pub(super) topbar_message: Signal<Option<String>>,
    pub(super) connection_badge: Signal<StatusBadge>,
    pub(super) worker_badge: Signal<StatusBadge>,
    pub(super) entries: Signal<Vec<TranscriptEntry>>,
    pub(super) pending_permissions: Signal<Vec<PendingPermission>>,
    pub(super) pending_action_busy: Signal<bool>,
}

#[derive(Clone, Copy)]
pub(super) struct SessionSidebarItemSignals {
    pub(super) is_renaming: Signal<bool>,
    pub(super) is_deleting: Signal<bool>,
    pub(super) is_saving_rename: Signal<bool>,
    pub(super) rename_action_disabled: Signal<bool>,
    pub(super) delete_action_disabled: Signal<bool>,
    pub(super) save_rename_disabled: Signal<bool>,
}

#[derive(Clone, Copy)]
pub(super) struct SessionSidebarItemCallbacks {
    pub(super) begin_rename: Callback<()>,
    pub(super) cancel_rename: Callback<()>,
    pub(super) commit_rename: Callback<()>,
    pub(super) delete_session: Callback<()>,
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

pub(super) fn session_composer_signals(
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

pub(super) fn session_shell_signals(signals: SessionSignals) -> SessionShellSignals {
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

pub(super) fn session_main_signals(signals: SessionSignals) -> SessionMainSignals {
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
) -> StatusBadge {
    connection_badge_state(session_status, has_connection_error)
}

fn main_worker_badge(
    session_status: SessionLifecycle,
    turn_state: TurnState,
    has_pending_permissions: bool,
) -> StatusBadge {
    worker_badge_state(session_status, turn_state, has_pending_permissions)
}

pub(super) fn session_sidebar_item_signals(
    session_id: String,
    is_current: bool,
    deleting_session_id: Signal<Option<String>>,
    delete_disabled: Signal<bool>,
    renaming_session_id: RwSignal<Option<String>>,
    saving_rename_session_id: Signal<Option<String>>,
    rename_draft: RwSignal<String>,
) -> SessionSidebarItemSignals {
    let is_renaming = sidebar_rw_session_match_signal(session_id.clone(), renaming_session_id);
    let is_deleting = sidebar_session_match_signal(session_id.clone(), deleting_session_id);
    let is_saving_rename = sidebar_session_match_signal(session_id, saving_rename_session_id);
    let rename_action_disabled =
        sidebar_rename_action_disabled_signal(is_deleting, saving_rename_session_id);
    let delete_action_disabled = sidebar_delete_action_disabled_signal(
        is_deleting,
        deleting_session_id,
        saving_rename_session_id,
        is_current,
        delete_disabled,
    );
    let save_rename_disabled = sidebar_save_rename_disabled_signal(is_saving_rename, rename_draft);

    SessionSidebarItemSignals {
        is_renaming,
        is_deleting,
        is_saving_rename,
        rename_action_disabled,
        delete_action_disabled,
        save_rename_disabled,
    }
}

fn sidebar_rw_session_match_signal(
    session_id: String,
    active_session_id: RwSignal<Option<String>>,
) -> Signal<bool> {
    Signal::derive(move || active_session_id.get().as_deref() == Some(session_id.as_str()))
}

fn sidebar_session_match_signal(
    session_id: String,
    active_session_id: Signal<Option<String>>,
) -> Signal<bool> {
    Signal::derive(move || active_session_id.get().as_deref() == Some(session_id.as_str()))
}

fn sidebar_rename_action_disabled_signal(
    is_deleting: Signal<bool>,
    saving_rename_session_id: Signal<Option<String>>,
) -> Signal<bool> {
    Signal::derive(move || is_deleting.get() || saving_rename_session_id.get().is_some())
}

fn sidebar_delete_action_disabled_signal(
    is_deleting: Signal<bool>,
    deleting_session_id: Signal<Option<String>>,
    saving_rename_session_id: Signal<Option<String>>,
    is_current: bool,
    delete_disabled: Signal<bool>,
) -> Signal<bool> {
    Signal::derive(move || {
        is_deleting.get()
            || deleting_session_id.get().is_some()
            || saving_rename_session_id.get().is_some()
            || (is_current && delete_disabled.get())
    })
}

fn sidebar_save_rename_disabled_signal(
    is_saving_rename: Signal<bool>,
    rename_draft: RwSignal<String>,
) -> Signal<bool> {
    Signal::derive(move || is_saving_rename.get() || rename_draft.get().trim().is_empty())
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
    pending_permissions: RwSignal<Vec<PendingPermission>>,
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
    use acp_contracts::{CompletionCandidate, CompletionKind, SessionListItem};

    use crate::domain::session::{PendingPermission, SessionLifecycle, TurnState};

    use super::*;

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn list_item(id: &str) -> SessionListItem {
        SessionListItem {
            id: id.to_string(),
            title: id.to_string(),
            status: acp_contracts::SessionStatus::Active,
            last_activity_at: chrono::Utc::now(),
        }
    }

    fn pending_permission(id: &str) -> PendingPermission {
        PendingPermission {
            request_id: id.to_string(),
            summary: format!("Permission for {id}"),
        }
    }

    fn slash_candidate(label: &str) -> CompletionCandidate {
        CompletionCandidate {
            label: label.to_string(),
            insert_text: label.to_string(),
            detail: "detail".to_string(),
            kind: CompletionKind::Command,
        }
    }

    // -----------------------------------------------------------------------
    // session_signals – initial values
    // -----------------------------------------------------------------------

    #[test]
    fn session_signals_starts_with_empty_loading_state() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            assert!(signals.entries.get().is_empty());
            assert!(signals.pending_permissions.get().is_empty());
            assert!(signals.action_error.get().is_none());
            assert!(signals.connection_error.get().is_none());
            assert!(signals.event_source.get().is_none());
            assert!(signals.stream_abort.get().is_none());
            assert_eq!(signals.session_status.get(), SessionLifecycle::Loading);
            assert_eq!(signals.turn_state.get(), TurnState::Idle);
            assert!(!signals.pending_action_busy.get());
            assert!(signals.draft.get().is_empty());
            assert!(signals.slash.candidates.get().is_empty());
            assert_eq!(signals.slash.selected_index.get(), 0);
            assert!(signals.list.items.get().is_empty());
            assert!(!signals.list.loaded.get());
            assert!(signals.list.error.get().is_none());
            assert!(signals.list.deleting_id.get().is_none());
            assert!(signals.list.renaming_id.get().is_none());
            assert!(signals.list.saving_rename_id.get().is_none());
            assert!(signals.list.rename_draft.get().is_empty());
            assert_eq!(signals.tool_activity_serial.get(), 0);
        });
    }

    // -----------------------------------------------------------------------
    // current_session_deleting_signal
    // -----------------------------------------------------------------------

    #[test]
    fn current_session_deleting_signal_tracks_deleting_id() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            let is_deleting = current_session_deleting_signal("session1".to_string(), signals);

            assert!(!is_deleting.get(), "no session is being deleted initially");

            signals.list.deleting_id.set(Some("other".to_string()));
            assert!(!is_deleting.get(), "a different session is being deleted");

            signals.list.deleting_id.set(Some("session1".to_string()));
            assert!(is_deleting.get(), "the target session is now being deleted");

            signals.list.deleting_id.set(None);
            assert!(!is_deleting.get(), "delete cleared");
        });
    }

    // -----------------------------------------------------------------------
    // session_composer_signals – disabled / status / cancel visibility
    // -----------------------------------------------------------------------

    #[test]
    fn composer_disabled_reflects_session_status_and_deleting_state() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            let not_deleting = Signal::derive(|| false);
            let composer = session_composer_signals(signals, not_deleting);

            // Loading → disabled
            assert!(composer.disabled.get());

            // Active + Idle → enabled
            signals.session_status.set(SessionLifecycle::Active);
            assert!(!composer.disabled.get());

            // Active + Submitting → disabled
            signals.turn_state.set(TurnState::Submitting);
            assert!(composer.disabled.get());

            signals.turn_state.set(TurnState::Idle);
        });
    }

    #[test]
    fn composer_disabled_when_current_session_is_being_deleted() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals.session_status.set(SessionLifecycle::Active);
            let is_deleting_signal = RwSignal::new(false);
            let composer =
                session_composer_signals(signals, Signal::derive(move || is_deleting_signal.get()));

            assert!(!composer.disabled.get());
            is_deleting_signal.set(true);
            assert!(composer.disabled.get());
        });
    }

    #[test]
    fn composer_status_message_matches_session_and_turn_state() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            let not_deleting = Signal::derive(|| false);
            let composer = session_composer_signals(signals, not_deleting);

            // Loading state
            assert_eq!(composer.status.get(), "Connecting...");

            // Active + Idle → empty
            signals.session_status.set(SessionLifecycle::Active);
            assert!(composer.status.get().is_empty());

            // Active + AwaitingReply
            signals.turn_state.set(TurnState::AwaitingReply);
            assert_eq!(composer.status.get(), "Waiting for response...");
        });
    }

    #[test]
    fn composer_cancel_visible_only_when_awaiting_reply_or_cancelling() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals.session_status.set(SessionLifecycle::Active);
            let not_deleting = Signal::derive(|| false);
            let composer = session_composer_signals(signals, not_deleting);

            assert!(!composer.cancel_visible.get());

            signals.turn_state.set(TurnState::AwaitingReply);
            assert!(composer.cancel_visible.get());

            signals.turn_state.set(TurnState::Cancelling);
            assert!(composer.cancel_visible.get());

            signals.turn_state.set(TurnState::Idle);
            assert!(!composer.cancel_visible.get());
        });
    }

    #[test]
    fn composer_cancel_hidden_when_pending_permissions_present() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals.session_status.set(SessionLifecycle::Active);
            signals.turn_state.set(TurnState::AwaitingReply);
            let not_deleting = Signal::derive(|| false);
            let composer = session_composer_signals(signals, not_deleting);

            assert!(composer.cancel_visible.get());

            signals
                .pending_permissions
                .set(vec![pending_permission("req1")]);
            assert!(!composer.cancel_visible.get());
        });
    }

    #[test]
    fn composer_cancel_busy_when_action_busy_deleting_or_cancelling() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals.session_status.set(SessionLifecycle::Active);
            let is_deleting_signal = RwSignal::new(false);
            let composer =
                session_composer_signals(signals, Signal::derive(move || is_deleting_signal.get()));

            assert!(!composer.cancel_busy.get());

            signals.pending_action_busy.set(true);
            assert!(composer.cancel_busy.get());
            signals.pending_action_busy.set(false);

            is_deleting_signal.set(true);
            assert!(composer.cancel_busy.get());
            is_deleting_signal.set(false);

            signals.turn_state.set(TurnState::Cancelling);
            assert!(composer.cancel_busy.get());
        });
    }

    #[test]
    fn composer_slash_palette_signals_forward_draft_and_candidate_values() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            let not_deleting = Signal::derive(|| false);
            let composer = session_composer_signals(signals, not_deleting);

            assert!(!composer.slash_palette_visible.get());
            assert!(composer.slash_candidates.get().is_empty());
            assert_eq!(composer.slash_selected_index.get(), 0);
            assert!(!composer.slash_apply_selected.get());

            // Slash prefix activates palette visibility
            signals.draft.set("/".to_string());
            assert!(composer.slash_palette_visible.get());

            signals.slash.candidates.set(vec![slash_candidate("/help")]);
            signals.slash.selected_index.set(0);
            assert_eq!(composer.slash_candidates.get().len(), 1);
            assert_eq!(composer.slash_selected_index.get(), 0);
        });
    }

    // -----------------------------------------------------------------------
    // session_shell_signals
    // -----------------------------------------------------------------------

    #[test]
    fn shell_signals_reflect_session_list_and_delete_disabled_state() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            let shell = session_shell_signals(signals);

            assert!(shell.sessions.get().is_empty());
            assert!(!shell.delete_disabled.get());

            signals
                .list
                .items
                .set(vec![list_item("s1"), list_item("s2")]);
            assert_eq!(shell.sessions.get().len(), 2);

            // Busy turn state disables delete
            signals.turn_state.set(TurnState::Submitting);
            assert!(shell.delete_disabled.get());
        });
    }

    #[test]
    fn restore_session_draft_applies_non_empty_stored_value() {
        let owner = Owner::new();
        owner.with(|| {
            let draft = RwSignal::new(String::new());

            apply_restored_session_draft(draft, "saved draft".to_string());

            assert_eq!(draft.get(), "saved draft");
        });
    }

    #[test]
    fn restore_and_persist_session_draft_are_host_safe() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals.draft.set("existing".to_string());

            restore_session_draft("session-1", signals);
            assert_eq!(signals.draft.get(), "existing");

            persist_session_draft_text("session-1", "draft");
            persist_session_draft("session-1".to_string(), signals.draft);
        });
    }

    // -----------------------------------------------------------------------
    // session_main_signals
    // -----------------------------------------------------------------------

    #[test]
    fn main_signals_reflect_session_status_and_topbar_messages() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            let main = session_main_signals(signals);

            assert_eq!(main.session_status.get(), SessionLifecycle::Loading);
            assert!(main.topbar_message.get().is_none());

            signals.action_error.set(Some("Action failed".to_string()));
            assert_eq!(main.topbar_message.get(), Some("Action failed".to_string()));

            // action_error takes priority over connection_error
            signals
                .connection_error
                .set(Some("Network issue".to_string()));
            assert_eq!(main.topbar_message.get(), Some("Action failed".to_string()));

            // With action_error cleared, connection_error appears
            signals.action_error.set(None);
            assert_eq!(main.topbar_message.get(), Some("Network issue".to_string()));
        });
    }

    #[test]
    fn main_signals_forward_entries_and_permissions() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            let main = session_main_signals(signals);

            assert!(main.entries.get().is_empty());
            assert!(main.pending_permissions.get().is_empty());
            assert!(!main.pending_action_busy.get());

            signals
                .pending_permissions
                .set(vec![pending_permission("req1")]);
            assert_eq!(main.pending_permissions.get().len(), 1);

            signals.pending_action_busy.set(true);
            assert!(main.pending_action_busy.get());
        });
    }

    #[test]
    fn main_badge_helpers_reflect_connection_and_worker_state() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            let main = session_main_signals(signals);

            assert_eq!(
                main_connection_badge(SessionLifecycle::Loading, false),
                main.connection_badge.get()
            );

            signals.session_status.set(SessionLifecycle::Active);
            signals.turn_state.set(TurnState::AwaitingReply);
            signals
                .pending_permissions
                .set(vec![pending_permission("req-1")]);

            assert_eq!(
                main_worker_badge(SessionLifecycle::Active, TurnState::AwaitingReply, true),
                main.worker_badge.get()
            );
        });
    }

    // -----------------------------------------------------------------------
    // session_sidebar_item_signals
    // -----------------------------------------------------------------------

    #[test]
    fn sidebar_item_is_renaming_tracks_renaming_id_rw_signal() {
        let owner = Owner::new();
        owner.with(|| {
            let renaming_id: RwSignal<Option<String>> = RwSignal::new(None);
            let saving_rename: RwSignal<Option<String>> = RwSignal::new(None);
            let deleting_id: RwSignal<Option<String>> = RwSignal::new(None);
            let delete_disabled = Signal::derive(|| false);

            let item = session_sidebar_item_signals(
                "s1".to_string(),
                false,
                Signal::derive(move || deleting_id.get()),
                delete_disabled,
                renaming_id,
                Signal::derive(move || saving_rename.get()),
                RwSignal::new(String::new()),
            );

            assert!(!item.is_renaming.get());
            renaming_id.set(Some("s1".to_string()));
            assert!(item.is_renaming.get());
            renaming_id.set(Some("other".to_string()));
            assert!(!item.is_renaming.get());
        });
    }

    #[test]
    fn sidebar_item_is_deleting_tracks_deleting_signal() {
        let owner = Owner::new();
        owner.with(|| {
            let renaming_id: RwSignal<Option<String>> = RwSignal::new(None);
            let saving_rename: RwSignal<Option<String>> = RwSignal::new(None);
            let deleting_id: RwSignal<Option<String>> = RwSignal::new(None);
            let delete_disabled = Signal::derive(|| false);

            let item = session_sidebar_item_signals(
                "s1".to_string(),
                false,
                Signal::derive(move || deleting_id.get()),
                delete_disabled,
                renaming_id,
                Signal::derive(move || saving_rename.get()),
                RwSignal::new(String::new()),
            );

            assert!(!item.is_deleting.get());
            deleting_id.set(Some("s1".to_string()));
            assert!(item.is_deleting.get());
        });
    }

    #[test]
    fn sidebar_item_rename_action_disabled_when_deleting_or_saving_rename() {
        let owner = Owner::new();
        owner.with(|| {
            let renaming_id: RwSignal<Option<String>> = RwSignal::new(None);
            let saving_rename: RwSignal<Option<String>> = RwSignal::new(None);
            let deleting_id: RwSignal<Option<String>> = RwSignal::new(None);
            let delete_disabled = Signal::derive(|| false);

            let item = session_sidebar_item_signals(
                "s1".to_string(),
                false,
                Signal::derive(move || deleting_id.get()),
                delete_disabled,
                renaming_id,
                Signal::derive(move || saving_rename.get()),
                RwSignal::new(String::new()),
            );

            assert!(!item.rename_action_disabled.get());

            deleting_id.set(Some("s1".to_string()));
            assert!(item.rename_action_disabled.get());
            deleting_id.set(None);

            saving_rename.set(Some("any".to_string()));
            assert!(item.rename_action_disabled.get());
        });
    }

    #[test]
    fn sidebar_item_delete_action_disabled_for_current_session_when_busy() {
        let owner = Owner::new();
        owner.with(|| {
            let renaming_id: RwSignal<Option<String>> = RwSignal::new(None);
            let saving_rename: RwSignal<Option<String>> = RwSignal::new(None);
            let deleting_id: RwSignal<Option<String>> = RwSignal::new(None);
            let delete_disabled: RwSignal<bool> = RwSignal::new(false);

            // is_current = true means the current session's delete button is affected
            let item = session_sidebar_item_signals(
                "s1".to_string(),
                true,
                Signal::derive(move || deleting_id.get()),
                Signal::derive(move || delete_disabled.get()),
                renaming_id,
                Signal::derive(move || saving_rename.get()),
                RwSignal::new(String::new()),
            );

            assert!(!item.delete_action_disabled.get());

            delete_disabled.set(true);
            assert!(item.delete_action_disabled.get());
        });
    }

    #[test]
    fn sidebar_item_save_rename_disabled_when_blank_draft_or_saving() {
        let owner = Owner::new();
        owner.with(|| {
            let renaming_id: RwSignal<Option<String>> = RwSignal::new(None);
            let saving_rename: RwSignal<Option<String>> = RwSignal::new(None);
            let deleting_id: RwSignal<Option<String>> = RwSignal::new(None);
            let delete_disabled = Signal::derive(|| false);
            let rename_draft: RwSignal<String> = RwSignal::new(String::new());

            let item = session_sidebar_item_signals(
                "s1".to_string(),
                false,
                Signal::derive(move || deleting_id.get()),
                delete_disabled,
                renaming_id,
                Signal::derive(move || saving_rename.get()),
                rename_draft,
            );

            // Blank draft → disabled
            assert!(item.save_rename_disabled.get());

            rename_draft.set("New title".to_string());
            assert!(!item.save_rename_disabled.get());

            saving_rename.set(Some("s1".to_string()));
            assert!(item.save_rename_disabled.get());
        });
    }
}
