use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Active,
    Closed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    User,
    Assistant,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConversationMessage {
    pub id: String,
    pub role: MessageRole,
    pub text: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionSnapshot {
    pub id: String,
    pub status: SessionStatus,
    pub latest_sequence: u64,
    pub messages: Vec<ConversationMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreateSessionResponse {
    pub session: SessionSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromptRequest {
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromptResponse {
    pub accepted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionHistoryResponse {
    pub session_id: String,
    pub messages: Vec<ConversationMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CloseSessionResponse {
    pub session: SessionSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HealthResponse {
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ErrorResponse {
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AssistantReplyRequest {
    pub session_id: String,
    pub prompt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AssistantReplyResponse {
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StreamEvent {
    pub sequence: u64,
    #[serde(flatten)]
    pub payload: StreamEventPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StreamEventPayload {
    SessionSnapshot { session: SessionSnapshot },
    ConversationMessage { message: ConversationMessage },
    SessionClosed { session_id: String, reason: String },
    Status { message: String },
}

impl StreamEvent {
    pub fn event_name(&self) -> &'static str {
        match &self.payload {
            StreamEventPayload::SessionSnapshot { .. } => "session.snapshot",
            StreamEventPayload::ConversationMessage { .. } => "conversation.message",
            StreamEventPayload::SessionClosed { .. } => "session.closed",
            StreamEventPayload::Status { .. } => "status",
        }
    }

    pub fn snapshot(snapshot: SessionSnapshot) -> Self {
        Self {
            sequence: snapshot.latest_sequence,
            payload: StreamEventPayload::SessionSnapshot { session: snapshot },
        }
    }

    pub fn status(sequence: u64, message: impl Into<String>) -> Self {
        Self {
            sequence,
            payload: StreamEventPayload::Status {
                message: message.into(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_closed_events_use_the_closed_event_name() {
        let event = StreamEvent {
            sequence: 7,
            payload: StreamEventPayload::SessionClosed {
                session_id: "s_test".to_string(),
                reason: "closed by user".to_string(),
            },
        };

        assert_eq!(event.event_name(), "session.closed");
    }

    #[test]
    fn status_helper_builds_a_status_event() {
        let event = StreamEvent::status(9, "mock request failed");

        assert_eq!(event.event_name(), "status");
        assert_eq!(event.sequence, 9);
        assert!(matches!(
            event.payload,
            StreamEventPayload::Status { message } if message == "mock request failed"
        ));
    }
}
