use serde::{Deserialize, Serialize};

use acp_contracts_messages::ConversationMessage;
use acp_contracts_permissions::PermissionRequest;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Active,
    Closed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionSnapshot {
    pub id: String,
    #[serde(default)]
    pub workspace_id: String,
    #[serde(default = "default_session_title")]
    pub title: String,
    pub status: SessionStatus,
    pub latest_sequence: u64,
    pub messages: Vec<ConversationMessage>,
    #[serde(default)]
    pub pending_permissions: Vec<PermissionRequest>,
    #[serde(default)]
    pub active_turn: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionListItem {
    pub id: String,
    #[serde(default)]
    pub workspace_id: String,
    #[serde(default = "default_session_title")]
    pub title: String,
    pub status: SessionStatus,
    pub last_activity_at: chrono::DateTime<chrono::Utc>,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct CreateSessionRequest {
    #[serde(default)]
    pub checkout_ref: Option<String>,
    #[serde(default)]
    pub agent_profile_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AgentProfileMode {
    #[default]
    Chroot,
    Host,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentProfile {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub mode: AgentProfileMode,
    pub command_argv: Vec<String>,
    #[serde(default)]
    pub env_allowlist: Vec<String>,
    #[serde(default = "default_agent_profile_timeout_seconds")]
    pub timeout_seconds: u64,
    #[serde(default = "default_agent_profile_run_uid")]
    pub run_uid: u32,
    #[serde(default = "default_agent_profile_run_gid")]
    pub run_gid: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentProfileListResponse {
    pub profiles: Vec<AgentProfile>,
    #[serde(default)]
    pub can_manage: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentProfileResponse {
    pub profile: AgentProfile,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeleteAgentProfileResponse {
    pub deleted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpsertAgentProfileRequest {
    pub name: String,
    #[serde(default)]
    pub mode: AgentProfileMode,
    pub command_argv: Vec<String>,
    #[serde(default)]
    pub env_allowlist: Vec<String>,
    #[serde(default = "default_agent_profile_timeout_seconds")]
    pub timeout_seconds: u64,
    #[serde(default = "default_agent_profile_run_uid")]
    pub run_uid: u32,
    #[serde(default = "default_agent_profile_run_gid")]
    pub run_gid: u32,
}

pub const fn default_agent_profile_timeout_seconds() -> u64 {
    30
}

pub const fn default_agent_profile_run_uid() -> u32 {
    65_534
}

pub const fn default_agent_profile_run_gid() -> u32 {
    65_534
}

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

#[cfg(test)]
mod tests {
    use super::{AgentProfileMode, CreateSessionRequest, SessionListItem, SessionSnapshot};

    #[test]
    fn session_snapshots_deserialize_default_titles_empty_permissions_and_workspace() {
        let snapshot: SessionSnapshot = serde_json::from_value(serde_json::json!({
            "id": "s_test",
            "status": "active",
            "latest_sequence": 1,
            "messages": [],
        }))
        .expect("session snapshots should deserialize");

        assert!(snapshot.workspace_id.is_empty());
        assert_eq!(snapshot.title, "New chat");
        assert!(snapshot.pending_permissions.is_empty());
        assert!(!snapshot.active_turn);
    }

    #[test]
    fn session_list_items_deserialize_default_title_and_workspace() {
        let item: SessionListItem = serde_json::from_value(serde_json::json!({
            "id": "s_test",
            "status": "active",
            "last_activity_at": "2026-04-17T01:00:00Z"
        }))
        .expect("session list items should deserialize");

        assert!(item.workspace_id.is_empty());
        assert_eq!(item.title, "New chat");
    }

    #[test]
    fn create_session_requests_default_optional_checkout_ref() {
        let request: CreateSessionRequest = serde_json::from_value(serde_json::json!({}))
            .expect("create requests should deserialize");

        assert_eq!(request.checkout_ref, None);
        assert_eq!(request.agent_profile_id, None);
    }

    #[test]
    fn agent_profile_mode_uses_snake_case() {
        let value = serde_json::to_value(AgentProfileMode::Chroot).expect("serialize mode");
        assert_eq!(value, serde_json::json!("chroot"));
        let value = serde_json::to_value(AgentProfileMode::Host).expect("serialize host mode");
        assert_eq!(value, serde_json::json!("host"));
        let mode: AgentProfileMode =
            serde_json::from_value(serde_json::json!("host")).expect("deserialize host mode");
        assert_eq!(mode, AgentProfileMode::Host);
    }

    #[test]
    fn upsert_agent_profile_request_defaults_runtime_fields() {
        let request: super::UpsertAgentProfileRequest = serde_json::from_value(serde_json::json!({
            "name": "OpenCode ACP",
            "command_argv": ["opencode", "acp"]
        }))
        .expect("profile request should deserialize");

        assert_eq!(request.mode, AgentProfileMode::Chroot);
        assert!(request.env_allowlist.is_empty());
        assert_eq!(request.timeout_seconds, 30);
        assert_eq!(request.run_uid, 65_534);
        assert_eq!(request.run_gid, 65_534);
    }
}
