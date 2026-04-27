use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceSummary {
    pub workspace_id: String,
    pub name: String,
    #[serde(default)]
    pub upstream_url: Option<String>,
    #[serde(default)]
    pub bootstrap_kind: Option<String>,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceDetail {
    pub workspace_id: String,
    pub name: String,
    #[serde(default)]
    pub upstream_url: Option<String>,
    #[serde(default)]
    pub credential_reference_id: Option<String>,
    #[serde(default)]
    pub bootstrap_kind: Option<String>,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreateWorkspaceRequest {
    pub name: String,
    pub upstream_url: String,
    #[serde(default)]
    pub credential_reference_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdateWorkspaceRequest {
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceListResponse {
    pub workspaces: Vec<WorkspaceSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceResponse {
    pub workspace: WorkspaceDetail,
}

pub type CreateWorkspaceResponse = WorkspaceResponse;
pub type UpdateWorkspaceResponse = WorkspaceResponse;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceBranch {
    pub name: String,
    pub ref_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceBranchListResponse {
    pub branches: Vec<WorkspaceBranch>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeleteWorkspaceResponse {
    pub deleted: bool,
}

#[cfg(test)]
mod tests {
    use super::{UpdateWorkspaceRequest, WorkspaceBranchListResponse, WorkspaceDetail};

    #[test]
    fn workspace_details_deserialize_optional_fields() {
        let detail: WorkspaceDetail = serde_json::from_value(serde_json::json!({
            "workspace_id": "w_test",
            "name": "Workspace",
            "status": "active",
            "created_at": "2026-04-17T01:00:00Z",
            "updated_at": "2026-04-17T01:00:00Z"
        }))
        .expect("workspace details should deserialize");

        assert_eq!(detail.workspace_id, "w_test");
        assert_eq!(detail.upstream_url, None);
        assert_eq!(detail.credential_reference_id, None);
        assert_eq!(detail.bootstrap_kind, None);
    }

    #[test]
    fn update_workspace_requests_default_optional_fields_to_none() {
        let request: UpdateWorkspaceRequest = serde_json::from_value(serde_json::json!({}))
            .expect("update requests should deserialize");

        assert_eq!(request.name, None);
    }

    #[test]
    fn workspace_branch_lists_round_trip() {
        let response: WorkspaceBranchListResponse = serde_json::from_value(serde_json::json!({
            "branches": [
                { "name": "main", "ref_name": "refs/heads/main" }
            ]
        }))
        .expect("branch list should deserialize");

        assert_eq!(response.branches.len(), 1);
        assert_eq!(response.branches[0].name, "main");
        assert_eq!(response.branches[0].ref_name, "refs/heads/main");
    }
}
