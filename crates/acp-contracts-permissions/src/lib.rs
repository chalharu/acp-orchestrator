use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PermissionRequest {
    pub request_id: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call: Option<ToolCallMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallMetadata {
    pub tool_call_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_input: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_output: Option<serde_json::Value>,
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
