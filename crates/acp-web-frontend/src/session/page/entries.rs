use acp_contracts_messages::{ConversationMessage, MessageRole};

use crate::components::transcript::{EntryRole, TranscriptEntry};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SessionEntry {
    pub(crate) id: String,
    pub(crate) role: SessionEntryRole,
    pub(crate) text: String,
}

impl SessionEntry {
    pub(crate) fn from_message(message: ConversationMessage) -> Self {
        Self {
            id: message.id,
            role: message.role.into(),
            text: message.text,
        }
    }

    pub(crate) fn status(id: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            role: SessionEntryRole::Status,
            text: text.into(),
        }
    }
}

impl From<SessionEntry> for TranscriptEntry {
    fn from(entry: SessionEntry) -> Self {
        Self {
            id: entry.id,
            role: entry.role.into(),
            text: entry.text,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SessionEntryRole {
    User,
    Assistant,
    Status,
}

impl From<SessionEntryRole> for EntryRole {
    fn from(role: SessionEntryRole) -> Self {
        match role {
            SessionEntryRole::User => Self::User,
            SessionEntryRole::Assistant => Self::Assistant,
            SessionEntryRole::Status => Self::Status,
        }
    }
}

impl From<MessageRole> for SessionEntryRole {
    fn from(role: MessageRole) -> Self {
        match role {
            MessageRole::User => Self::User,
            MessageRole::Assistant => Self::Assistant,
        }
    }
}

#[cfg(test)]
mod tests {
    use acp_contracts_messages::{ConversationMessage, MessageRole};
    use chrono::{TimeZone, Utc};

    use crate::components::transcript::{EntryRole, TranscriptEntry};

    use super::{SessionEntry, SessionEntryRole};

    #[test]
    fn session_entry_builds_from_messages_and_status() {
        let message_entry = SessionEntry::from_message(ConversationMessage {
            id: "m1".to_string(),
            role: MessageRole::Assistant,
            text: "hello".to_string(),
            created_at: Utc.with_ymd_and_hms(2026, 4, 17, 1, 0, 0).unwrap(),
        });

        assert_eq!(message_entry.id, "m1");
        assert_eq!(message_entry.role, SessionEntryRole::Assistant);
        assert_eq!(message_entry.text, "hello");

        assert_eq!(
            SessionEntryRole::from(MessageRole::User),
            SessionEntryRole::User
        );
        assert_eq!(
            SessionEntry::status("status-1", "done"),
            SessionEntry {
                id: "status-1".to_string(),
                role: SessionEntryRole::Status,
                text: "done".to_string(),
            }
        );
    }

    #[test]
    fn session_entries_convert_into_transcript_entries_for_every_role() {
        let user_entry = SessionEntry::from_message(ConversationMessage {
            id: "m2".to_string(),
            role: MessageRole::User,
            text: "prompt".to_string(),
            created_at: Utc.with_ymd_and_hms(2026, 4, 17, 1, 0, 2).unwrap(),
        });
        let status_entry = SessionEntry::status("status-2", "done");
        let assistant_entry = SessionEntry::from_message(ConversationMessage {
            id: "m3".to_string(),
            role: MessageRole::Assistant,
            text: "reply".to_string(),
            created_at: Utc.with_ymd_and_hms(2026, 4, 17, 1, 0, 3).unwrap(),
        });

        let user_transcript = TranscriptEntry::from(user_entry);
        let assistant_transcript = TranscriptEntry::from(assistant_entry);
        let status_transcript = TranscriptEntry::from(status_entry);

        assert_eq!(user_transcript.id, "m2");
        assert_eq!(user_transcript.role, EntryRole::User);
        assert_eq!(user_transcript.text, "prompt");
        assert_eq!(assistant_transcript.role, EntryRole::Assistant);
        assert_eq!(status_transcript.role, EntryRole::Status);
    }
}
