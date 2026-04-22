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
    #[serde(default = "default_session_title")]
    pub title: String,
    pub status: SessionStatus,
    pub latest_sequence: u64,
    pub messages: Vec<ConversationMessage>,
    #[serde(default)]
    pub pending_permissions: Vec<PermissionRequest>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionListItem {
    pub id: String,
    #[serde(default = "default_session_title")]
    pub title: String,
    pub status: SessionStatus,
    pub last_activity_at: DateTime<Utc>,
}

fn default_session_title() -> String {
    "New chat".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionListResponse {
    pub sessions: Vec<SessionListItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionResponse {
    pub session: SessionSnapshot,
}

pub type CreateSessionResponse = SessionResponse;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RenameSessionRequest {
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RenameSessionResponse {
    pub session: SessionSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeleteSessionResponse {
    pub deleted: bool,
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
pub struct PermissionRequest {
    pub request_id: String,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PermissionDecision {
    Approve,
    Deny,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResolvePermissionRequest {
    pub decision: PermissionDecision,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResolvePermissionResponse {
    pub request_id: String,
    pub decision: PermissionDecision,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CancelTurnResponse {
    pub cancelled: bool,
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
pub struct LocalAccount {
    pub user_id: String,
    pub username: String,
    pub is_admin: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthStatusResponse {
    pub bootstrap_required: bool,
    pub account: Option<LocalAccount>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BootstrapRegistrationRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BootstrapRegistrationResponse {
    pub account: LocalAccount,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SignInRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SignInResponse {
    pub account: LocalAccount,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccountListResponse {
    pub current_user_id: String,
    pub accounts: Vec<LocalAccount>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreateAccountRequest {
    pub username: String,
    pub password: String,
    pub is_admin: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreateAccountResponse {
    pub account: LocalAccount,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdateAccountRequest {
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub is_admin: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdateAccountResponse {
    pub account: LocalAccount,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeleteAccountResponse {
    pub deleted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompletionKind {
    Command,
    Parameter,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompletionCandidate {
    pub label: String,
    pub insert_text: String,
    pub detail: String,
    pub kind: CompletionKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SlashCompletionsResponse {
    pub candidates: Vec<CompletionCandidate>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlashCommand {
    Help,
    Quit,
    Cancel,
    Approve,
    Deny,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlashCommandSpec {
    pub command: SlashCommand,
    pub name: &'static str,
    pub label: &'static str,
    pub insert_text: &'static str,
    pub detail: &'static str,
}

pub const SLASH_COMMAND_SPECS: &[SlashCommandSpec] = &[
    SlashCommandSpec {
        command: SlashCommand::Help,
        name: "/help",
        label: "/help",
        insert_text: "/help",
        detail: "Show available slash commands",
    },
    SlashCommandSpec {
        command: SlashCommand::Quit,
        name: "/quit",
        label: "/quit",
        insert_text: "/quit",
        detail: "Exit chat",
    },
    SlashCommandSpec {
        command: SlashCommand::Cancel,
        name: "/cancel",
        label: "/cancel",
        insert_text: "/cancel",
        detail: "Cancel the running turn",
    },
    SlashCommandSpec {
        command: SlashCommand::Approve,
        name: "/approve",
        label: "/approve <request-id>",
        insert_text: "/approve ",
        detail: "Approve a pending permission request",
    },
    SlashCommandSpec {
        command: SlashCommand::Deny,
        name: "/deny",
        label: "/deny <request-id>",
        insert_text: "/deny ",
        detail: "Deny a pending permission request",
    },
];

impl SlashCommand {
    pub fn spec(self) -> &'static SlashCommandSpec {
        SLASH_COMMAND_SPECS
            .iter()
            .find(|spec| spec.command == self)
            .expect("every slash command must have a corresponding spec")
    }

    pub fn takes_request_id(self) -> bool {
        matches!(self, Self::Approve | Self::Deny)
    }
}

pub fn parse_slash_command(name: &str) -> Option<SlashCommand> {
    SLASH_COMMAND_SPECS
        .iter()
        .find(|spec| spec.name == name)
        .map(|spec| spec.command)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlashCompletionQuery<'a> {
    Commands {
        prefix: &'a str,
    },
    RequestId {
        command: SlashCommand,
        prefix: &'a str,
    },
}

pub fn classify_slash_completion_prefix(prefix: &str) -> Option<SlashCompletionQuery<'_>> {
    let normalized = prefix.trim_start();
    if normalized.is_empty() || !normalized.starts_with('/') {
        return None;
    }

    if let Some((name, argument_prefix)) = normalized.split_once(' ') {
        let command = parse_slash_command(name)?;
        if !command.takes_request_id() {
            return None;
        }

        let argument_prefix = argument_prefix.trim_start();
        if argument_prefix.chars().any(char::is_whitespace) {
            return None;
        }

        return Some(SlashCompletionQuery::RequestId {
            command,
            prefix: argument_prefix,
        });
    }

    SLASH_COMMAND_SPECS
        .iter()
        .any(|spec| spec.name.starts_with(normalized))
        .then_some(SlashCompletionQuery::Commands { prefix: normalized })
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
    PermissionRequested { request: PermissionRequest },
    SessionClosed { session_id: String, reason: String },
    Status { message: String },
}

impl StreamEvent {
    pub fn event_name(&self) -> &'static str {
        match &self.payload {
            StreamEventPayload::SessionSnapshot { .. } => "session.snapshot",
            StreamEventPayload::ConversationMessage { .. } => "conversation.message",
            StreamEventPayload::PermissionRequested { .. } => "tool.permission.requested",
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
    fn permission_requested_events_use_the_permission_event_name() {
        let event = StreamEvent {
            sequence: 8,
            payload: StreamEventPayload::PermissionRequested {
                request: PermissionRequest {
                    request_id: "req_1".to_string(),
                    summary: "read_text_file README.md".to_string(),
                },
            },
        };

        assert_eq!(event.event_name(), "tool.permission.requested");
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

    #[test]
    fn completion_candidates_serialize_kind_in_snake_case() {
        let payload = serde_json::to_value(CompletionCandidate {
            label: "/approve <request-id>".to_string(),
            insert_text: "/approve ".to_string(),
            detail: "Approve a pending permission request".to_string(),
            kind: CompletionKind::Command,
        })
        .expect("completion candidates should serialize");

        assert_eq!(payload["kind"], "command");
    }

    #[test]
    fn slash_completion_queries_only_allow_known_command_shapes() {
        assert!(matches!(
            classify_slash_completion_prefix("/ap"),
            Some(SlashCompletionQuery::Commands { prefix: "/ap" })
        ));
        assert!(matches!(
            classify_slash_completion_prefix("  /approve req_"),
            Some(SlashCompletionQuery::RequestId {
                command: SlashCommand::Approve,
                prefix: "req_",
            })
        ));
        assert!(classify_slash_completion_prefix("/approve req_1 extra").is_none());
        assert!(classify_slash_completion_prefix("/home/alice").is_none());
        assert!(classify_slash_completion_prefix("/quit now").is_none());
    }

    #[test]
    fn session_snapshots_deserialize_default_titles_and_empty_permissions() {
        let snapshot: SessionSnapshot = serde_json::from_value(serde_json::json!({
            "id": "s_test",
            "status": "active",
            "latest_sequence": 1,
            "messages": [],
        }))
        .expect("session snapshots should deserialize");

        assert_eq!(snapshot.title, "New chat");
        assert!(snapshot.pending_permissions.is_empty());
    }

    #[test]
    fn slash_command_specs_cover_labels_and_request_id_requirements() {
        assert_eq!(SlashCommand::Approve.spec().label, "/approve <request-id>");
        assert_eq!(SlashCommand::Deny.spec().insert_text, "/deny ");
        assert!(SlashCommand::Approve.takes_request_id());
        assert!(!SlashCommand::Help.takes_request_id());
        assert_eq!(parse_slash_command("/cancel"), Some(SlashCommand::Cancel));
    }
}
