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
    let stored_draft = load_draft(session_id);
    if !stored_draft.is_empty() {
        signals.draft.set(stored_draft);
    }
}

pub(super) fn persist_session_draft(session_id: String, draft: RwSignal<String>) {
    Effect::new(move |_| {
        save_draft(&session_id, &draft.get());
    });
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
            connection_badge_state(session_status.get(), connection_error.get().is_some())
        }),
        worker_badge: Signal::derive(move || {
            worker_badge_state(
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
