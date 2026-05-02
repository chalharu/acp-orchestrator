use serde::{Deserialize, Serialize};

use acp_contracts_messages::ConversationMessage;
use acp_contracts_permissions::{PermissionRequest, ToolCallMetadata};
use acp_contracts_sessions::SessionSnapshot;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StreamEvent {
    pub sequence: u64,
    #[serde(flatten)]
    pub payload: StreamEventPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StreamEventPayload {
    SessionSnapshot {
        session: SessionSnapshot,
    },
    ConversationMessage {
        message: ConversationMessage,
        #[serde(default)]
        partial: bool,
    },
    PermissionRequested {
        request: PermissionRequest,
    },
    ToolCall {
        call: ToolCallMetadata,
    },
    ToolCallUpdate {
        update: ToolCallMetadata,
    },
    SessionClosed {
        session_id: String,
        reason: String,
    },
    Status {
        message: String,
    },
}

impl StreamEvent {
    pub fn event_name(&self) -> &'static str {
        match &self.payload {
            StreamEventPayload::SessionSnapshot { .. } => "session.snapshot",
            StreamEventPayload::ConversationMessage { .. } => "conversation.message",
            StreamEventPayload::PermissionRequested { .. } => "tool.permission.requested",
            StreamEventPayload::ToolCall { .. } => "tool.call",
            StreamEventPayload::ToolCallUpdate { .. } => "tool.call.update",
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
    use super::{StreamEvent, StreamEventPayload};
    use acp_contracts_permissions::{PermissionRequest, ToolCallMetadata};

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
    fn permission_requested_events_use_the_permission_event_name() {
        let event = StreamEvent {
            sequence: 8,
            payload: StreamEventPayload::PermissionRequested {
                request: PermissionRequest {
                    request_id: "req_1".to_string(),
                    summary: "read_text_file README.md".to_string(),
                    tool_call: None,
                },
            },
        };

        assert_eq!(event.event_name(), "tool.permission.requested");
    }

    #[test]
    fn tool_events_use_tool_event_names() {
        let call = ToolCallMetadata {
            tool_call_id: "tool_1".to_string(),
            title: Some("Read".to_string()),
            kind: Some("read".to_string()),
            status: Some("pending".to_string()),
            raw_input: None,
            raw_output: None,
        };

        assert_eq!(
            StreamEvent {
                sequence: 9,
                payload: StreamEventPayload::ToolCall { call: call.clone() },
            }
            .event_name(),
            "tool.call"
        );
        assert_eq!(
            StreamEvent {
                sequence: 10,
                payload: StreamEventPayload::ToolCallUpdate { update: call },
            }
            .event_name(),
            "tool.call.update"
        );
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
