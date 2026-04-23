use acp_contracts_messages::{ConversationMessage, MessageRole};

use crate::components::transcript::{EntryRole, TranscriptItem};

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

impl TranscriptItem for SessionEntry {
    fn transcript_id(&self) -> &str {
        &self.id
    }

    fn transcript_role(&self) -> EntryRole {
        match self.role {
            SessionEntryRole::User => EntryRole::User,
            SessionEntryRole::Assistant => EntryRole::Assistant,
            SessionEntryRole::Status => EntryRole::Status,
        }
    }

    fn transcript_text(&self) -> &str {
        &self.text
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SessionEntryRole {
    User,
    Assistant,
    Status,
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

    use crate::components::transcript::{EntryRole, TranscriptItem};

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
    fn transcript_item_impl_exposes_entry_fields_for_every_role() {
        let user_entry = SessionEntry::from_message(ConversationMessage {
            id: "m2".to_string(),
            role: MessageRole::User,
            text: "prompt".to_string(),
            created_at: Utc.with_ymd_and_hms(2026, 4, 17, 1, 0, 2).unwrap(),
        });
        let status_entry = SessionEntry::status("status-2", "done");

        assert_eq!(user_entry.transcript_id(), "m2");
        assert_eq!(user_entry.transcript_role(), EntryRole::User);
        assert_eq!(user_entry.transcript_text(), "prompt");
        assert_eq!(
            SessionEntry::from_message(ConversationMessage {
                id: "m3".to_string(),
                role: MessageRole::Assistant,
                text: "reply".to_string(),
                created_at: Utc.with_ymd_and_hms(2026, 4, 17, 1, 0, 3).unwrap(),
            })
            .transcript_role(),
            EntryRole::Assistant
        );
        assert_eq!(status_entry.transcript_role(), EntryRole::Status);
    }
}
