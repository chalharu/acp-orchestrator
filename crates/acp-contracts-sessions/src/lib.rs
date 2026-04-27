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
    use super::{CreateSessionRequest, SessionListItem, SessionSnapshot};

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
    }
}
