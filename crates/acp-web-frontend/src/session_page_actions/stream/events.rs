#![cfg_attr(not(target_family = "wasm"), allow(dead_code))]

use acp_contracts_messages::{ConversationMessage, MessageRole};
use acp_contracts_permissions::PermissionRequest;
use acp_contracts_sessions::{SessionSnapshot, SessionStatus};
use acp_contracts_slash::CompletionCandidate;
use acp_contracts_stream::{StreamEvent, StreamEventPayload};
use leptos::prelude::*;

use crate::session_activity::tool_activity_text;
use crate::session_lifecycle::{SessionLifecycle, TurnState, session_end_message};
use crate::session_page_bootstrap::session_bootstrap_from_snapshot;
use crate::session_page_entries::SessionEntry;
use crate::session_page_signals::SessionSignals;
use crate::session_state::{
    should_apply_snapshot_turn_state, should_release_turn_state, turn_state_for_snapshot,
};

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
    signals
        .pending_permissions
        .set(bootstrap.pending_permissions);
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
    push_status_entry(
        signals.entries,
        sequence,
        session_end_message(Some(&reason)),
    );
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

#[cfg(test)]
mod tests {
    use acp_contracts_messages::{ConversationMessage, MessageRole};
    use acp_contracts_permissions::PermissionRequest;
    use acp_contracts_sessions::{SessionListItem, SessionSnapshot, SessionStatus};
    use acp_contracts_slash::{CompletionCandidate, CompletionKind};
    use acp_contracts_stream::{StreamEvent, StreamEventPayload};
    use chrono::{TimeZone, Utc};
    use leptos::prelude::*;

    use super::{
        handle_sse_event, next_tool_activity_id, push_status_entry, push_tool_activity_entry,
    };
    use crate::session_lifecycle::{SessionLifecycle, TurnState};
    use crate::session_page_signals::session_signals;

    fn message(id: &str, role: MessageRole, text: &str) -> ConversationMessage {
        ConversationMessage {
            id: id.to_string(),
            role,
            text: text.to_string(),
            created_at: Utc.with_ymd_and_hms(2026, 4, 17, 1, 0, 0).unwrap(),
        }
    }

    fn permission(id: &str) -> PermissionRequest {
        PermissionRequest {
            request_id: id.to_string(),
            summary: format!("Permission for {id}"),
        }
    }

    fn snapshot(status: SessionStatus, permissions: Vec<PermissionRequest>) -> SessionSnapshot {
        SessionSnapshot {
            id: "session-1".to_string(),
            title: "Session".to_string(),
            status,
            latest_sequence: 4,
            messages: vec![message("assistant-1", MessageRole::Assistant, "hello")],
            pending_permissions: permissions,
        }
    }

    fn list_item(id: &str) -> SessionListItem {
        SessionListItem {
            id: id.to_string(),
            title: id.to_string(),
            status: SessionStatus::Active,
            last_activity_at: Utc.with_ymd_and_hms(2026, 4, 17, 1, 0, 0).unwrap(),
        }
    }

    fn command(label: &str) -> CompletionCandidate {
        CompletionCandidate {
            label: label.to_string(),
            insert_text: label.to_string(),
            detail: "detail".to_string(),
            kind: CompletionKind::Command,
        }
    }

    #[test]
    fn session_snapshot_event_updates_entries_permissions_and_turn_state() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            handle_sse_event(
                StreamEvent {
                    sequence: 1,
                    payload: StreamEventPayload::SessionSnapshot {
                        session: snapshot(SessionStatus::Active, vec![permission("perm-1")]),
                    },
                },
                signals,
            );

            assert_eq!(signals.session_status.get(), SessionLifecycle::Active);
            assert_eq!(signals.turn_state.get(), TurnState::AwaitingPermission);
            assert_eq!(signals.entries.get().len(), 1);
            assert_eq!(signals.pending_permissions.get().len(), 1);
        });
    }

    #[test]
    fn conversation_message_event_deduplicates_entries_and_releases_turn_state() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals.turn_state.set(TurnState::AwaitingReply);

            let event = StreamEvent {
                sequence: 2,
                payload: StreamEventPayload::ConversationMessage {
                    message: message("assistant-1", MessageRole::Assistant, "hello"),
                },
            };
            handle_sse_event(event.clone(), signals);
            handle_sse_event(event, signals);

            assert_eq!(signals.entries.get().len(), 1);
            assert_eq!(signals.turn_state.get(), TurnState::Idle);
        });
    }

    #[test]
    fn permission_and_status_events_append_activity_only_once() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            let permission_event = StreamEvent {
                sequence: 3,
                payload: StreamEventPayload::PermissionRequested {
                    request: permission("perm-1"),
                },
            };
            handle_sse_event(permission_event.clone(), signals);
            handle_sse_event(permission_event, signals);
            assert_eq!(signals.pending_permissions.get().len(), 1);
            assert_eq!(signals.turn_state.get(), TurnState::AwaitingPermission);
            assert_eq!(signals.entries.get().len(), 1);

            handle_sse_event(
                StreamEvent {
                    sequence: 4,
                    payload: StreamEventPayload::Status {
                        message: "Working".to_string(),
                    },
                },
                signals,
            );
            handle_sse_event(
                StreamEvent {
                    sequence: 4,
                    payload: StreamEventPayload::Status {
                        message: "Working".to_string(),
                    },
                },
                signals,
            );
            assert_eq!(signals.entries.get().len(), 2);
        });
    }

    #[test]
    fn status_updates_release_busy_turns_and_blank_activity_is_ignored() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals.turn_state.set(TurnState::AwaitingReply);

            handle_sse_event(
                StreamEvent {
                    sequence: 6,
                    payload: StreamEventPayload::Status {
                        message: "Done".to_string(),
                    },
                },
                signals,
            );
            assert_eq!(signals.turn_state.get(), TurnState::Idle);

            push_tool_activity_entry(signals, "blank".to_string(), "", "", Vec::new());
            push_status_entry(signals.entries, 6, "Done".to_string());
            assert_eq!(signals.entries.get().len(), 1);
        });
    }

    #[test]
    fn session_closed_event_marks_session_closed_and_clears_busy_state() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals.list.items.set(vec![list_item("session-1")]);
            signals.pending_permissions.set(vec![permission("perm-1")]);
            signals.pending_action_busy.set(true);
            signals.turn_state.set(TurnState::AwaitingReply);

            handle_sse_event(
                StreamEvent {
                    sequence: 5,
                    payload: StreamEventPayload::SessionClosed {
                        session_id: "session-1".to_string(),
                        reason: "closed by user".to_string(),
                    },
                },
                signals,
            );

            assert_eq!(signals.session_status.get(), SessionLifecycle::Closed);
            assert_eq!(signals.turn_state.get(), TurnState::Idle);
            assert!(signals.pending_permissions.get().is_empty());
            assert!(!signals.pending_action_busy.get());
            assert_eq!(signals.list.items.get()[0].status, SessionStatus::Closed);
        });
    }

    #[test]
    fn tool_activity_helpers_increment_ids_and_ignore_blank_status_text() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();

            assert_eq!(next_tool_activity_id(signals, "help"), "help-1");
            assert_eq!(next_tool_activity_id(signals, "help"), "help-2");

            push_status_entry(signals.entries, 1, "   ".to_string());
            assert!(signals.entries.get().is_empty());

            push_tool_activity_entry(
                signals,
                "tool-1".to_string(),
                "Title",
                "Detail",
                vec![command("/help")],
            );
            assert_eq!(signals.entries.get().len(), 1);
            assert!(signals.entries.get()[0].text.contains("Commands:"));
        });
    }
}
