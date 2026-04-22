#![cfg_attr(not(target_family = "wasm"), allow(dead_code))]

use acp_contracts_messages::{ConversationMessage, MessageRole};
use acp_contracts_permissions::PermissionRequest;
use acp_contracts_sessions::{SessionSnapshot, SessionStatus};
use acp_contracts_slash::CompletionCandidate;
use acp_contracts_stream::{StreamEvent, StreamEventPayload};
use leptos::prelude::*;

use crate::session_activity::tool_activity_text;
use crate::session_lifecycle::{SessionLifecycle, TurnState, session_end_message};
use crate::session_state::{
    should_apply_snapshot_turn_state, should_release_turn_state, turn_state_for_snapshot,
};

use super::super::super::bootstrap::session_bootstrap_from_snapshot;
use super::super::super::entries::SessionEntry;
use super::super::super::state::SessionSignals;

pub(super) fn handle_sse_event(event: StreamEvent, signals: SessionSignals) {
    let StreamEvent { sequence, payload } = event;

    match payload {
        StreamEventPayload::SessionSnapshot { session } => apply_session_snapshot(session, signals),
        StreamEventPayload::ConversationMessage { message } => {
            apply_conversation_message(message, signals)
        }
        StreamEventPayload::PermissionRequested { request } => {
            apply_permission_request(request, signals)
        }
        StreamEventPayload::SessionClosed { session_id, reason } => {
            apply_session_closed(sequence, session_id, reason, signals)
        }
        StreamEventPayload::Status { message } => apply_status_update(sequence, message, signals),
    }
}

fn apply_session_snapshot(session: SessionSnapshot, signals: SessionSignals) {
    let bootstrap = session_bootstrap_from_snapshot(session);
    signals.session_status.set(bootstrap.session_status);
    if should_apply_snapshot_turn_state(signals.turn_state.get_untracked()) {
        signals
            .turn_state
            .set(turn_state_for_snapshot(&bootstrap.pending_permissions));
    }
    signals.pending_permissions.set(bootstrap.pending_permissions);
    signals.entries.set(bootstrap.entries);
}

fn apply_conversation_message(message: ConversationMessage, signals: SessionSignals) {
    let is_assistant_message = matches!(message.role, MessageRole::Assistant);
    let mut appended = false;
    signals.entries.update(|current_entries| {
        if !current_entries.iter().any(|entry| entry.id == message.id) {
            appended = true;
            current_entries.push(SessionEntry::from_message(message));
        }
    });
    if appended
        && is_assistant_message
        && should_release_turn_state(signals.turn_state.get_untracked())
    {
        signals.turn_state.set(TurnState::Idle);
    }
}

fn apply_permission_request(request: PermissionRequest, signals: SessionSignals) {
    let request_id = request.request_id.clone();
    let summary = request.summary.clone();
    signals.pending_permissions.update(|current_permissions| {
        if !current_permissions
            .iter()
            .any(|current_permission| current_permission.request_id == request.request_id)
        {
            current_permissions.push(request.clone());
        }
    });
    signals.turn_state.set(TurnState::AwaitingPermission);
    push_tool_activity_entry(
        signals,
        format!("permission-{request_id}"),
        "Permission required",
        summary,
        Vec::new(),
    );
}

fn apply_session_closed(
    sequence: u64,
    session_id: String,
    reason: String,
    signals: SessionSignals,
) {
    signals.session_status.set(SessionLifecycle::Closed);
    signals.turn_state.set(TurnState::Idle);
    signals.pending_permissions.set(Vec::new());
    signals.pending_action_busy.set(false);
    signals
        .list
        .items
        .update(|sessions| mark_session_closed(sessions, &session_id));
    push_status_entry(signals.entries, sequence, session_end_message(Some(&reason)));
}

fn apply_status_update(sequence: u64, message: String, signals: SessionSignals) {
    if should_release_turn_state(signals.turn_state.get_untracked()) {
        signals.turn_state.set(TurnState::Idle);
    }
    push_status_entry(signals.entries, sequence, message);
}

fn push_status_entry(
    entries: leptos::prelude::RwSignal<Vec<SessionEntry>>,
    sequence: u64,
    text: String,
) {
    if text.trim().is_empty() {
        return;
    }

    let entry_id = format!("status-{sequence}");
    entries.update(|current_entries| {
        if current_entries.iter().any(|entry| entry.id == entry_id) {
            return;
        }

        current_entries.push(SessionEntry::status(entry_id.clone(), text.clone()));
    });
}

fn push_activity_entry(
    entries: leptos::prelude::RwSignal<Vec<SessionEntry>>,
    id: String,
    text: String,
) {
    if text.trim().is_empty() {
        return;
    }

    entries.update(|current_entries| {
        if current_entries.iter().any(|entry| entry.id == id) {
            return;
        }

        current_entries.push(SessionEntry::status(id, text));
    });
}

pub(crate) fn next_tool_activity_id(signals: SessionSignals, prefix: &str) -> String {
    let next = signals.tool_activity_serial.get_untracked() + 1;
    signals.tool_activity_serial.set(next);
    format!("{prefix}-{next}")
}

pub(crate) fn push_tool_activity_entry(
    signals: SessionSignals,
    id: String,
    title: impl Into<String>,
    detail: impl Into<String>,
    commands: Vec<CompletionCandidate>,
) {
    let title = title.into();
    let detail = detail.into();
    push_activity_entry(
        signals.entries,
        format!("activity-{id}"),
        tool_activity_text(&title, &detail, &commands),
    );
}

fn mark_session_closed(sessions: &mut [acp_contracts_sessions::SessionListItem], session_id: &str) {
    if let Some(session) = sessions.iter_mut().find(|session| session.id == session_id) {
        session.status = SessionStatus::Closed;
    }
}
