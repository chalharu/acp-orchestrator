use acp_contracts_permissions::PermissionRequest;
use acp_contracts_sessions::SessionSnapshot;

use crate::session_lifecycle::{session_status_label, SessionLifecycle, CLOSED_SESSION_MESSAGE};

use super::entries::{SessionEntry, SessionEntryRole};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SessionBootstrap {
    pub(crate) entries: Vec<SessionEntry>,
    pub(crate) pending_permissions: Vec<PermissionRequest>,
    pub(crate) session_status: SessionLifecycle,
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
        .map(SessionEntry::from_message)
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
) -> Vec<PermissionRequest> {
    pending_permissions
}

pub(crate) fn push_bootstrap_closed_status_entry(entries: &mut Vec<SessionEntry>) {
    if entries.iter().any(|entry| {
        matches!(entry.role, SessionEntryRole::Status) && entry.text == CLOSED_SESSION_MESSAGE
    }) {
        return;
    }

    entries.push(SessionEntry::status(
        "status-session-ended",
        CLOSED_SESSION_MESSAGE,
    ));
}

#[cfg(test)]
mod tests {
    use acp_contracts_messages::{ConversationMessage, MessageRole};
    use acp_contracts_permissions::PermissionRequest;
    use acp_contracts_sessions::{SessionResponse, SessionSnapshot, SessionStatus};
    use chrono::{TimeZone, Utc};

    use super::{
        pending_permissions_to_items, push_bootstrap_closed_status_entry,
        session_bootstrap_from_snapshot, SessionLifecycle,
    };
    use crate::session::page::entries::{SessionEntry, SessionEntryRole};
    use crate::session_lifecycle::CLOSED_SESSION_MESSAGE;

    fn sample_session_bootstrap_response() -> SessionResponse {
        SessionResponse {
            session: SessionSnapshot {
                id: "s_123".to_string(),
                title: "My test session".to_string(),
                status: SessionStatus::Closed,
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

    #[test]
    fn session_bootstrap_from_snapshot_maps_messages_and_permissions() {
        let bootstrap = session_bootstrap_from_snapshot(sample_session_bootstrap_response().session);

        assert_eq!(bootstrap.session_status, SessionLifecycle::Closed);
        assert_eq!(bootstrap.entries.len(), 3);
        assert_eq!(bootstrap.entries[0].id, "m_user");
        assert!(matches!(bootstrap.entries[0].role, SessionEntryRole::User));
        assert_eq!(bootstrap.entries[1].id, "m_assistant");
        assert!(matches!(
            bootstrap.entries[1].role,
            SessionEntryRole::Assistant
        ));
        assert!(matches!(bootstrap.entries[2].role, SessionEntryRole::Status));
        assert_eq!(bootstrap.entries[2].text, CLOSED_SESSION_MESSAGE);
        assert_eq!(
            bootstrap.pending_permissions,
            vec![PermissionRequest {
                request_id: "req_1".to_string(),
                summary: "read README.md".to_string(),
            }]
        );
    }

    #[test]
    fn bootstrap_helpers_cover_open_and_duplicate_status_cases() {
        let pending_permissions = vec![PermissionRequest {
            request_id: "req_2".to_string(),
            summary: "inspect src".to_string(),
        }];
        assert_eq!(
            pending_permissions_to_items(pending_permissions),
            vec![PermissionRequest {
                request_id: "req_2".to_string(),
                summary: "inspect src".to_string(),
            }]
        );

        let mut entries = vec![SessionEntry::status(
            "status-session-ended",
            CLOSED_SESSION_MESSAGE,
        )];
        push_bootstrap_closed_status_entry(&mut entries);
        assert_eq!(entries.len(), 1);
    }
}
