use acp_contracts::{
    CompletionCandidate, ConversationMessage, MessageRole, PermissionRequest, SessionListItem,
    SessionSnapshot, SessionStatus,
};

use super::routing::app_session_path;
use super::transcript::{EntryRole, TranscriptEntry};

pub(crate) const CLOSED_SESSION_MESSAGE: &str = "Conversation ended.";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PendingPermission {
    pub request_id: String,
    pub summary: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BadgeTone {
    Neutral,
    Success,
    Warning,
    Danger,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StatusBadge {
    pub label: &'static str,
    pub value: &'static str,
    pub tone: BadgeTone,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SessionLifecycle {
    Loading,
    Active,
    Closed,
    Unavailable,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TurnState {
    Idle,
    Submitting,
    AwaitingReply,
    AwaitingPermission,
    Cancelling,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SessionBootstrap {
    pub entries: Vec<TranscriptEntry>,
    pub pending_permissions: Vec<PendingPermission>,
    pub session_status: SessionLifecycle,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SidebarSession {
    pub id: String,
    pub href: String,
    pub title: String,
    pub activity_label: String,
    pub is_current: bool,
    pub is_closed: bool,
}

pub(crate) fn session_bootstrap_from_snapshot(session: SessionSnapshot) -> SessionBootstrap {
    let SessionSnapshot {
        status,
        messages,
        pending_permissions,
        ..
    } = session;
    let session_status = session_status_label(status);
    let mut entries = messages
        .into_iter()
        .map(message_to_entry)
        .collect::<Vec<_>>();
    if matches!(session_status, SessionLifecycle::Closed) {
        push_bootstrap_closed_status_entry(&mut entries);
    }

    SessionBootstrap {
        entries,
        pending_permissions: pending_permissions_to_items(pending_permissions),
        session_status,
    }
}

pub(crate) fn pending_permissions_to_items(
    pending_permissions: Vec<PermissionRequest>,
) -> Vec<PendingPermission> {
    pending_permissions
        .into_iter()
        .map(|request| PendingPermission {
            request_id: request.request_id,
            summary: request.summary,
        })
        .collect()
}

pub(crate) fn session_end_message(reason: Option<&str>) -> String {
    let Some(reason) = reason.map(str::trim) else {
        return CLOSED_SESSION_MESSAGE.to_string();
    };
    if reason.is_empty() || reason == "closed by user" {
        CLOSED_SESSION_MESSAGE.to_string()
    } else {
        reason.to_string()
    }
}

pub(crate) fn message_to_entry(message: ConversationMessage) -> TranscriptEntry {
    TranscriptEntry {
        id: message.id,
        role: message_role(message.role),
        text: message.text,
    }
}

pub(crate) fn message_role(role: MessageRole) -> EntryRole {
    match role {
        MessageRole::User => EntryRole::User,
        MessageRole::Assistant => EntryRole::Assistant,
    }
}

pub(crate) fn session_status_label(status: SessionStatus) -> SessionLifecycle {
    match status {
        SessionStatus::Active => SessionLifecycle::Active,
        SessionStatus::Closed => SessionLifecycle::Closed,
    }
}

pub(crate) fn push_bootstrap_closed_status_entry(entries: &mut Vec<TranscriptEntry>) {
    if entries.iter().any(|entry| {
        matches!(entry.role, EntryRole::Status) && entry.text == CLOSED_SESSION_MESSAGE
    }) {
        return;
    }

    entries.push(TranscriptEntry {
        id: "status-session-ended".to_string(),
        role: EntryRole::Status,
        text: CLOSED_SESSION_MESSAGE.to_string(),
    });
}

pub(crate) fn session_action_busy(
    turn_state: TurnState,
    pending_action_busy: bool,
    action_in_progress: bool,
) -> bool {
    pending_action_busy || action_in_progress || turn_state != TurnState::Idle
}

pub(crate) fn session_composer_disabled(
    session_status: SessionLifecycle,
    turn_state: TurnState,
    current_session_deleting: bool,
) -> bool {
    current_session_deleting
        || session_status != SessionLifecycle::Active
        || turn_state != TurnState::Idle
}

pub(crate) fn session_composer_status_message(
    session_status: SessionLifecycle,
    turn_state: TurnState,
    current_session_deleting: bool,
) -> String {
    if current_session_deleting {
        return "Deleting session...".to_string();
    }
    match turn_state {
        TurnState::Submitting | TurnState::AwaitingReply => "Waiting for response...".to_string(),
        TurnState::AwaitingPermission => {
            "Resolve the pending request before sending another message.".to_string()
        }
        TurnState::Cancelling => "Cancelling...".to_string(),
        TurnState::Idle => match session_status {
            SessionLifecycle::Active => String::new(),
            SessionLifecycle::Closed => "This conversation has ended.".to_string(),
            SessionLifecycle::Loading => "Connecting...".to_string(),
            SessionLifecycle::Unavailable | SessionLifecycle::Error => {
                "Session unavailable. Start a fresh chat.".to_string()
            }
        },
    }
}

pub(crate) fn session_composer_cancel_visible(
    turn_state: TurnState,
    has_pending_permissions: bool,
    current_session_deleting: bool,
) -> bool {
    !current_session_deleting
        && !has_pending_permissions
        && matches!(turn_state, TurnState::AwaitingReply | TurnState::Cancelling)
}

pub(crate) fn should_apply_snapshot_turn_state(current: TurnState) -> bool {
    matches!(current, TurnState::Idle | TurnState::AwaitingPermission)
}

pub(crate) fn should_release_turn_state(current: TurnState) -> bool {
    matches!(current, TurnState::AwaitingReply | TurnState::Cancelling)
}

pub(crate) fn turn_state_for_snapshot(pending_permissions: &[PendingPermission]) -> TurnState {
    if pending_permissions.is_empty() {
        TurnState::Idle
    } else {
        TurnState::AwaitingPermission
    }
}

pub(crate) fn tool_activity_text(
    title: &str,
    detail: &str,
    commands: &[CompletionCandidate],
) -> String {
    let mut lines = Vec::new();

    let title = title.trim();
    if !title.is_empty() {
        lines.push(title.to_string());
    }

    let detail = detail.trim();
    if !detail.is_empty() {
        lines.push(detail.to_string());
    }

    if !commands.is_empty() {
        lines.push("Commands:".to_string());
        lines.extend(commands.iter().map(format_tool_activity_command));
    }

    lines.join("\n")
}

pub(crate) fn format_tool_activity_command(command: &CompletionCandidate) -> String {
    let detail = command.detail.trim();
    if detail.is_empty() {
        format!("- {}", command.label)
    } else {
        format!("- {} — {}", command.label, detail)
    }
}

pub(crate) fn connection_badge_state(
    session_status: SessionLifecycle,
    has_connection_error: bool,
) -> StatusBadge {
    match session_status {
        SessionLifecycle::Loading => StatusBadge {
            label: "Connection",
            value: "connecting",
            tone: BadgeTone::Neutral,
        },
        SessionLifecycle::Active if has_connection_error => StatusBadge {
            label: "Connection",
            value: "reconnecting",
            tone: BadgeTone::Warning,
        },
        SessionLifecycle::Active => StatusBadge {
            label: "Connection",
            value: "live",
            tone: BadgeTone::Success,
        },
        SessionLifecycle::Closed => StatusBadge {
            label: "Connection",
            value: "ended",
            tone: BadgeTone::Neutral,
        },
        SessionLifecycle::Unavailable | SessionLifecycle::Error => StatusBadge {
            label: "Connection",
            value: "unavailable",
            tone: BadgeTone::Danger,
        },
    }
}

pub(crate) fn worker_badge_state(
    session_status: SessionLifecycle,
    turn_state: TurnState,
    has_pending_permissions: bool,
) -> StatusBadge {
    match session_status {
        SessionLifecycle::Loading => StatusBadge {
            label: "Worker",
            value: "starting",
            tone: BadgeTone::Neutral,
        },
        SessionLifecycle::Unavailable | SessionLifecycle::Error => StatusBadge {
            label: "Worker",
            value: "unavailable",
            tone: BadgeTone::Danger,
        },
        SessionLifecycle::Closed => StatusBadge {
            label: "Worker",
            value: "stopped",
            tone: BadgeTone::Neutral,
        },
        SessionLifecycle::Active if has_pending_permissions => StatusBadge {
            label: "Worker",
            value: "permission",
            tone: BadgeTone::Warning,
        },
        SessionLifecycle::Active => match turn_state {
            TurnState::Submitting | TurnState::AwaitingReply => StatusBadge {
                label: "Worker",
                value: "running",
                tone: BadgeTone::Success,
            },
            TurnState::Cancelling => StatusBadge {
                label: "Worker",
                value: "cancelling",
                tone: BadgeTone::Warning,
            },
            TurnState::AwaitingPermission => StatusBadge {
                label: "Worker",
                value: "permission",
                tone: BadgeTone::Warning,
            },
            TurnState::Idle => StatusBadge {
                label: "Worker",
                value: "idle",
                tone: BadgeTone::Neutral,
            },
        },
    }
}

pub(crate) fn status_badge_class(badge: StatusBadge) -> &'static str {
    match badge.tone {
        BadgeTone::Neutral => "status-badge status-badge--neutral",
        BadgeTone::Success => "status-badge status-badge--success",
        BadgeTone::Warning => "status-badge status-badge--warning",
        BadgeTone::Danger => "status-badge status-badge--danger",
    }
}

pub(crate) fn sidebar_sessions(
    sessions: &[SessionListItem],
    current_session_id: &str,
) -> Vec<SidebarSession> {
    sessions
        .iter()
        .map(|session| SidebarSession {
            href: app_session_path(&session.id),
            title: if session.title.is_empty() {
                "New chat".to_string()
            } else {
                session.title.clone()
            },
            activity_label: sidebar_session_activity_label(session),
            id: session.id.clone(),
            is_current: session.id == current_session_id,
            is_closed: matches!(session.status, SessionStatus::Closed),
        })
        .collect()
}

pub(crate) fn sidebar_session_activity_label(session: &SessionListItem) -> String {
    format!(
        "Updated {}",
        session.last_activity_at.format("%Y-%m-%d %H:%M UTC")
    )
}

pub(crate) fn session_sidebar_status_label(is_closed: bool) -> &'static str {
    if is_closed { "closed" } else { "active" }
}

pub(crate) fn session_sidebar_status_pill_class(is_closed: bool) -> &'static str {
    if is_closed {
        "session-sidebar__status-pill session-sidebar__status-pill--neutral"
    } else {
        "session-sidebar__status-pill session-sidebar__status-pill--success"
    }
}

pub(crate) fn mark_session_closed(sessions: &mut [SessionListItem], session_id: &str) {
    if let Some(session) = sessions.iter_mut().find(|session| session.id == session_id) {
        session.status = SessionStatus::Closed;
    }
}

pub(crate) fn remove_session_from_list(sessions: &mut Vec<SessionListItem>, session_id: &str) {
    sessions.retain(|session| session.id != session_id);
}

pub(crate) fn next_session_destination(sessions: &[SessionListItem]) -> String {
    sessions
        .first()
        .map(|session| app_session_path(&session.id))
        .unwrap_or_else(|| "/app/".to_string())
}

pub(crate) fn rename_session_in_list(
    sessions: &mut [SessionListItem],
    session_id: &str,
    title: String,
) {
    if let Some(session) = sessions.iter_mut().find(|session| session.id == session_id) {
        session.title = title;
    }
}

#[cfg(test)]
mod tests {
    use acp_contracts::{SessionResponse, SessionSnapshot, SessionStatus};
    use chrono::{TimeZone, Utc};

    use super::{
        BadgeTone, CLOSED_SESSION_MESSAGE, EntryRole, PendingPermission, SessionLifecycle,
        SidebarSession, StatusBadge, TurnState, connection_badge_state, format_tool_activity_command,
        mark_session_closed, message_role, next_session_destination, pending_permissions_to_items,
        push_bootstrap_closed_status_entry, remove_session_from_list, rename_session_in_list,
        session_action_busy, session_bootstrap_from_snapshot, session_composer_cancel_visible,
        session_composer_disabled, session_composer_status_message, session_end_message,
        session_sidebar_status_label, session_sidebar_status_pill_class, session_status_label,
        should_apply_snapshot_turn_state, should_release_turn_state, sidebar_session_activity_label,
        sidebar_sessions, status_badge_class, tool_activity_text, turn_state_for_snapshot,
        worker_badge_state,
    };
    use acp_contracts::{
        CompletionCandidate, CompletionKind, ConversationMessage, MessageRole, PermissionRequest,
        SessionListItem,
    };

    fn sample_session_bootstrap_response() -> SessionResponse {
        SessionResponse {
            session: SessionSnapshot {
                id: "s_123".to_string(),
                title: "My test session".to_string(),
                status: acp_contracts::SessionStatus::Closed,
                latest_sequence: 8,
                messages: vec![
                    ConversationMessage {
                        id: "m_user".to_string(),
                        role: MessageRole::User,
                        text: "hello".to_string(),
                        created_at: Utc.with_ymd_and_hms(2026, 4, 17, 1, 0, 0).unwrap(),
                    },
                    ConversationMessage {
                        id: "m_assistant".to_string(),
                        role: MessageRole::Assistant,
                        text: "world".to_string(),
                        created_at: Utc.with_ymd_and_hms(2026, 4, 17, 1, 0, 1).unwrap(),
                    },
                ],
                pending_permissions: vec![PermissionRequest {
                    request_id: "req_1".to_string(),
                    summary: "read README.md".to_string(),
                }],
            },
        }
    }

    fn sample_list_item(id: &str, title: &str, status: SessionStatus, minute: u32) -> SessionListItem {
        SessionListItem {
            id: id.to_string(),
            title: title.to_string(),
            status,
            last_activity_at: Utc.with_ymd_and_hms(2026, 4, 17, 1, minute, 0).unwrap(),
        }
    }

    #[test]
    fn session_composer_helpers_match_turn_state() {
        assert!(session_composer_disabled(
            SessionLifecycle::Active,
            TurnState::AwaitingReply,
            false,
        ));
        assert_eq!(
            session_composer_status_message(
                SessionLifecycle::Active,
                TurnState::AwaitingPermission,
                false,
            ),
            "Resolve the pending request before sending another message."
        );
        assert_eq!(
            session_composer_status_message(SessionLifecycle::Closed, TurnState::Idle, false),
            "This conversation has ended."
        );
        assert!(session_composer_cancel_visible(
            TurnState::AwaitingReply,
            false,
            false,
        ));
    }

    #[test]
    fn turn_state_helpers_match_permission_state() {
        assert!(should_release_turn_state(TurnState::AwaitingReply));
        assert_eq!(turn_state_for_snapshot(&[]), TurnState::Idle);
        assert_eq!(
            turn_state_for_snapshot(&[PendingPermission {
                request_id: "req_1".to_string(),
                summary: "read README.md".to_string(),
            }]),
            TurnState::AwaitingPermission
        );
        assert!(session_action_busy(TurnState::Submitting, false, false));
    }

    #[test]
    fn session_end_message_normalizes_empty_and_default_reasons() {
        assert_eq!(session_end_message(None), CLOSED_SESSION_MESSAGE);
        assert_eq!(session_end_message(Some("   ")), CLOSED_SESSION_MESSAGE);
        assert_eq!(
            session_end_message(Some(" closed by user ")),
            CLOSED_SESSION_MESSAGE
        );
        assert_eq!(session_end_message(Some(" timeout ")), "timeout");
    }

    #[test]
    fn session_bootstrap_from_snapshot_maps_messages_and_permissions() {
        let bootstrap = session_bootstrap_from_snapshot(sample_session_bootstrap_response().session);

        assert_eq!(bootstrap.session_status, SessionLifecycle::Closed);
        assert_eq!(bootstrap.entries.len(), 3);
        assert_eq!(bootstrap.entries[0].id, "m_user");
        assert!(matches!(bootstrap.entries[0].role, EntryRole::User));
        assert_eq!(bootstrap.entries[1].id, "m_assistant");
        assert!(matches!(bootstrap.entries[1].role, EntryRole::Assistant));
        assert!(matches!(bootstrap.entries[2].role, EntryRole::Status));
        assert_eq!(bootstrap.entries[2].text, CLOSED_SESSION_MESSAGE);
        assert_eq!(
            bootstrap.pending_permissions,
            vec![PendingPermission {
                request_id: "req_1".to_string(),
                summary: "read README.md".to_string(),
            }]
        );
    }

    #[test]
    fn session_bootstrap_helpers_cover_open_and_duplicate_status_cases() {
        let pending_permissions = vec![PermissionRequest {
            request_id: "req_2".to_string(),
            summary: "inspect src".to_string(),
        }];
        assert_eq!(
            pending_permissions_to_items(pending_permissions),
            vec![PendingPermission {
                request_id: "req_2".to_string(),
                summary: "inspect src".to_string(),
            }]
        );

        assert_eq!(session_status_label(SessionStatus::Active), SessionLifecycle::Active);
        assert_eq!(message_role(MessageRole::User), EntryRole::User);
        assert_eq!(message_role(MessageRole::Assistant), EntryRole::Assistant);

        let mut entries = vec![super::TranscriptEntry {
            id: "status-session-ended".to_string(),
            role: EntryRole::Status,
            text: CLOSED_SESSION_MESSAGE.to_string(),
        }];
        push_bootstrap_closed_status_entry(&mut entries);
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn sidebar_sessions_preserve_backend_order_and_labels() {
        let sessions = vec![
            sample_list_item("s_newest", "Task about rust", SessionStatus::Active, 0),
            sample_list_item("s_closed", "Old exploration", SessionStatus::Closed, 1),
        ];

        assert_eq!(
            sidebar_sessions(&sessions, "s_closed"),
            vec![
                SidebarSession {
                    id: "s_newest".to_string(),
                    href: "/app/sessions/s_newest".to_string(),
                    title: "Task about rust".to_string(),
                    activity_label: "Updated 2026-04-17 01:00 UTC".to_string(),
                    is_current: false,
                    is_closed: false,
                },
                SidebarSession {
                    id: "s_closed".to_string(),
                    href: "/app/sessions/s_closed".to_string(),
                    title: "Old exploration".to_string(),
                    activity_label: "Updated 2026-04-17 01:01 UTC".to_string(),
                    is_current: true,
                    is_closed: true,
                },
            ]
        );
    }

    #[test]
    fn sidebar_helpers_cover_empty_titles_and_fallback_destination() {
        let session = sample_list_item("s_empty", "", SessionStatus::Active, 2);
        assert_eq!(
            sidebar_session_activity_label(&session),
            "Updated 2026-04-17 01:02 UTC"
        );
        assert_eq!(
            sidebar_sessions(&[session], "other")[0].title,
            "New chat".to_string()
        );
        assert_eq!(next_session_destination(&[]), "/app/");
        assert_eq!(
            session_sidebar_status_pill_class(false),
            "session-sidebar__status-pill session-sidebar__status-pill--success"
        );
    }

    #[test]
    fn sidebar_session_mutators_update_only_the_target_item() {
        let mut sessions = vec![
            sample_list_item("s_a", "Original A", SessionStatus::Active, 0),
            sample_list_item("s_b", "Original B", SessionStatus::Active, 1),
        ];

        rename_session_in_list(&mut sessions, "s_a", "Renamed A".to_string());
        mark_session_closed(&mut sessions, "s_b");
        remove_session_from_list(&mut sessions, "missing");

        assert_eq!(sessions[0].title, "Renamed A");
        assert_eq!(sessions[1].status, SessionStatus::Closed);
        assert_eq!(next_session_destination(&sessions), "/app/sessions/s_a");
    }

    #[test]
    fn remove_session_from_list_removes_matching_items() {
        let mut sessions = vec![
            sample_list_item("s_a", "A", SessionStatus::Active, 0),
            sample_list_item("s_b", "B", SessionStatus::Active, 1),
        ];

        remove_session_from_list(&mut sessions, "s_a");

        assert_eq!(sessions, vec![sample_list_item("s_b", "B", SessionStatus::Active, 1)]);
    }

    #[test]
    fn composer_status_and_cancel_helpers_cover_all_branches() {
        assert_eq!(
            session_composer_status_message(SessionLifecycle::Active, TurnState::Idle, true),
            "Deleting session..."
        );
        assert_eq!(
            session_composer_status_message(
                SessionLifecycle::Active,
                TurnState::Submitting,
                false,
            ),
            "Waiting for response..."
        );
        assert_eq!(
            session_composer_status_message(
                SessionLifecycle::Active,
                TurnState::AwaitingReply,
                false,
            ),
            "Waiting for response..."
        );
        assert_eq!(
            session_composer_status_message(SessionLifecycle::Active, TurnState::Cancelling, false),
            "Cancelling..."
        );
        assert_eq!(
            session_composer_status_message(SessionLifecycle::Active, TurnState::Idle, false),
            ""
        );
        assert_eq!(
            session_composer_status_message(SessionLifecycle::Loading, TurnState::Idle, false),
            "Connecting..."
        );
        assert_eq!(
            session_composer_status_message(SessionLifecycle::Unavailable, TurnState::Idle, false),
            "Session unavailable. Start a fresh chat."
        );
        assert_eq!(
            session_composer_status_message(SessionLifecycle::Error, TurnState::Idle, false),
            "Session unavailable. Start a fresh chat."
        );

        assert!(!session_composer_cancel_visible(
            TurnState::AwaitingReply,
            true,
            false,
        ));
        assert!(!session_composer_cancel_visible(
            TurnState::AwaitingReply,
            false,
            true,
        ));
        assert!(session_composer_cancel_visible(
            TurnState::Cancelling,
            false,
            false,
        ));
    }

    #[test]
    fn turn_state_helpers_cover_snapshot_application_and_idle_paths() {
        assert!(should_apply_snapshot_turn_state(TurnState::Idle));
        assert!(should_apply_snapshot_turn_state(TurnState::AwaitingPermission));
        assert!(!should_apply_snapshot_turn_state(TurnState::Submitting));
        assert!(!should_release_turn_state(TurnState::Idle));
        assert_eq!(turn_state_for_snapshot(&[]), TurnState::Idle);
        assert!(!session_action_busy(TurnState::Idle, false, false));
    }

    #[test]
    fn tool_activity_helpers_trim_inputs_and_format_commands() {
        let command_with_detail = CompletionCandidate {
            label: "/help".to_string(),
            insert_text: "/help".to_string(),
            detail: " show commands ".to_string(),
            kind: CompletionKind::Command,
        };
        let command_without_detail = CompletionCandidate {
            label: "/quit".to_string(),
            insert_text: "/quit".to_string(),
            detail: "   ".to_string(),
            kind: CompletionKind::Command,
        };

        assert_eq!(
            tool_activity_text(
                "  Run command  ",
                "  Inspect repo  ",
                &[command_with_detail.clone(), command_without_detail.clone()],
            ),
            "Run command\nInspect repo\nCommands:\n- /help — show commands\n- /quit"
        );
        assert_eq!(
            tool_activity_text("   ", "   ", &[]),
            ""
        );
        assert_eq!(
            format_tool_activity_command(&command_without_detail),
            "- /quit"
        );
    }

    #[test]
    fn badge_helpers_reflect_live_and_reconnecting_states() {
        let connection = connection_badge_state(SessionLifecycle::Active, true);
        let worker = worker_badge_state(SessionLifecycle::Active, TurnState::AwaitingReply, false);

        assert_eq!(connection.value, "reconnecting");
        assert_eq!(connection.tone, BadgeTone::Warning);
        assert_eq!(worker.value, "running");
        assert_eq!(worker.tone, BadgeTone::Success);
        assert_eq!(session_sidebar_status_label(false), "active");
        assert_eq!(
            session_sidebar_status_pill_class(true),
            "session-sidebar__status-pill session-sidebar__status-pill--neutral"
        );
    }

    #[test]
    fn badge_helpers_cover_remaining_states_and_css_classes() {
        assert_eq!(
            connection_badge_state(SessionLifecycle::Loading, false),
            StatusBadge {
                label: "Connection",
                value: "connecting",
                tone: BadgeTone::Neutral,
            }
        );
        assert_eq!(
            connection_badge_state(SessionLifecycle::Active, false),
            StatusBadge {
                label: "Connection",
                value: "live",
                tone: BadgeTone::Success,
            }
        );
        assert_eq!(
            connection_badge_state(SessionLifecycle::Closed, false),
            StatusBadge {
                label: "Connection",
                value: "ended",
                tone: BadgeTone::Neutral,
            }
        );
        assert_eq!(
            connection_badge_state(SessionLifecycle::Error, false),
            StatusBadge {
                label: "Connection",
                value: "unavailable",
                tone: BadgeTone::Danger,
            }
        );

        assert_eq!(
            worker_badge_state(SessionLifecycle::Loading, TurnState::Idle, false),
            StatusBadge {
                label: "Worker",
                value: "starting",
                tone: BadgeTone::Neutral,
            }
        );
        assert_eq!(
            worker_badge_state(SessionLifecycle::Unavailable, TurnState::Idle, false),
            StatusBadge {
                label: "Worker",
                value: "unavailable",
                tone: BadgeTone::Danger,
            }
        );
        assert_eq!(
            worker_badge_state(SessionLifecycle::Closed, TurnState::Idle, false),
            StatusBadge {
                label: "Worker",
                value: "stopped",
                tone: BadgeTone::Neutral,
            }
        );
        assert_eq!(
            worker_badge_state(SessionLifecycle::Active, TurnState::Idle, true),
            StatusBadge {
                label: "Worker",
                value: "permission",
                tone: BadgeTone::Warning,
            }
        );
        assert_eq!(
            worker_badge_state(SessionLifecycle::Active, TurnState::Cancelling, false),
            StatusBadge {
                label: "Worker",
                value: "cancelling",
                tone: BadgeTone::Warning,
            }
        );
        assert_eq!(
            worker_badge_state(SessionLifecycle::Active, TurnState::AwaitingPermission, false),
            StatusBadge {
                label: "Worker",
                value: "permission",
                tone: BadgeTone::Warning,
            }
        );
        assert_eq!(
            worker_badge_state(SessionLifecycle::Active, TurnState::Idle, false),
            StatusBadge {
                label: "Worker",
                value: "idle",
                tone: BadgeTone::Neutral,
            }
        );

        assert_eq!(
            status_badge_class(StatusBadge {
                label: "x",
                value: "y",
                tone: BadgeTone::Neutral,
            }),
            "status-badge status-badge--neutral"
        );
        assert_eq!(
            status_badge_class(StatusBadge {
                label: "x",
                value: "y",
                tone: BadgeTone::Success,
            }),
            "status-badge status-badge--success"
        );
        assert_eq!(
            status_badge_class(StatusBadge {
                label: "x",
                value: "y",
                tone: BadgeTone::Warning,
            }),
            "status-badge status-badge--warning"
        );
        assert_eq!(
            status_badge_class(StatusBadge {
                label: "x",
                value: "y",
                tone: BadgeTone::Danger,
            }),
            "status-badge status-badge--danger"
        );
    }
}
