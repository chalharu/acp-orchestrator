#![cfg_attr(not(target_family = "wasm"), allow(dead_code))]

use acp_contracts_sessions::{
    CreateSessionRequest, CreateSessionResponse, SessionListItem, SessionListResponse,
};
use acp_contracts_workspaces::{
    CreateWorkspaceRequest, CreateWorkspaceResponse, DeleteWorkspaceResponse,
    UpdateWorkspaceRequest, UpdateWorkspaceResponse, WorkspaceBranch,
    WorkspaceBranchListResponse, WorkspaceDetail, WorkspaceListResponse, WorkspaceSummary,
};
#[cfg(target_family = "wasm")]
use gloo_net::http::Request;

#[cfg(not(target_family = "wasm"))]
use super::encode_component;
#[cfg(target_family = "wasm")]
use super::response_error_message;
#[cfg(target_family = "wasm")]
use super::{csrf_token, encode_component, patch_json_with_csrf, post_json_with_csrf};

const WORKSPACES_URL: &str = "/api/v1/workspaces";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WorkspaceSessionCreateError {
    NotFound(String),
    Other(String),
}

impl WorkspaceSessionCreateError {
    pub(crate) fn into_message(self) -> String {
        match self {
            Self::NotFound(message) | Self::Other(message) => message,
        }
    }
}

impl std::fmt::Display for WorkspaceSessionCreateError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(message) | Self::Other(message) => formatter.write_str(message),
        }
    }
}

#[cfg(target_family = "wasm")]
pub(crate) async fn list_workspaces() -> Result<Vec<WorkspaceSummary>, String> {
    let response = Request::get(WORKSPACES_URL)
        .send()
        .await
        .map_err(|error| error.to_string())?;

    if !response.ok() {
        return Err(response_error_message(response, "List workspaces failed").await);
    }

    let listed: WorkspaceListResponse = response.json().await.map_err(|error| error.to_string())?;
    Ok(listed.workspaces)
}

#[cfg(not(target_family = "wasm"))]
pub(crate) async fn list_workspaces() -> Result<Vec<WorkspaceSummary>, String> {
    Err(non_wasm_api_error("GET", WORKSPACES_URL))
}

#[cfg(target_family = "wasm")]
pub(crate) async fn create_workspace(
    name: &str,
    upstream_url: String,
) -> Result<WorkspaceDetail, String> {
    let body = create_workspace_body(name, upstream_url)?;
    let response = post_json_with_csrf(WORKSPACES_URL, body).await?;
    if !response.ok() {
        return Err(response_error_message(response, "Create workspace failed").await);
    }
    let payload: CreateWorkspaceResponse =
        response.json().await.map_err(|error| error.to_string())?;
    Ok(payload.workspace)
}

#[cfg(not(target_family = "wasm"))]
pub(crate) async fn create_workspace(
    name: &str,
    upstream_url: String,
) -> Result<WorkspaceDetail, String> {
    let _ = create_workspace_body(name, upstream_url)?;
    Err(non_wasm_api_error("POST", WORKSPACES_URL))
}

#[cfg(target_family = "wasm")]
pub(crate) async fn update_workspace(
    workspace_id: &str,
    name: Option<String>,
) -> Result<WorkspaceDetail, String> {
    let body = update_workspace_body(name)?;
    let response = patch_json_with_csrf(&workspace_url(workspace_id), body).await?;
    if !response.ok() {
        return Err(response_error_message(response, "Update workspace failed").await);
    }
    let payload: UpdateWorkspaceResponse =
        response.json().await.map_err(|error| error.to_string())?;
    Ok(payload.workspace)
}

#[cfg(not(target_family = "wasm"))]
pub(crate) async fn update_workspace(
    workspace_id: &str,
    name: Option<String>,
) -> Result<WorkspaceDetail, String> {
    let _ = update_workspace_body(name)?;
    Err(non_wasm_api_error("PATCH", &workspace_url(workspace_id)))
}

#[cfg(target_family = "wasm")]
pub(crate) async fn delete_workspace(
    workspace_id: &str,
) -> Result<DeleteWorkspaceResponse, String> {
    let csrf = csrf_token();
    let response = Request::delete(&workspace_url(workspace_id))
        .header("x-csrf-token", &csrf)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.ok() {
        return Err(response_error_message(response, "Delete workspace failed").await);
    }
    response.json().await.map_err(|error| error.to_string())
}

#[cfg(not(target_family = "wasm"))]
pub(crate) async fn delete_workspace(
    workspace_id: &str,
) -> Result<DeleteWorkspaceResponse, String> {
    Err(non_wasm_api_error("DELETE", &workspace_url(workspace_id)))
}

#[cfg(target_family = "wasm")]
pub(crate) async fn create_workspace_session(
    workspace_id: &str,
    checkout_ref: Option<String>,
) -> Result<String, WorkspaceSessionCreateError> {
    let response = post_json_with_csrf(
        &workspace_sessions_url(workspace_id),
        create_workspace_session_body(checkout_ref).map_err(WorkspaceSessionCreateError::Other)?,
    )
    .await
    .map_err(WorkspaceSessionCreateError::Other)?;
    if response.status() == 404 {
        return Err(WorkspaceSessionCreateError::NotFound(
            response_error_message(response, "Create workspace session failed").await,
        ));
    }
    if !response.ok() {
        return Err(WorkspaceSessionCreateError::Other(
            response_error_message(response, "Create workspace session failed").await,
        ));
    }
    let payload: CreateSessionResponse = response
        .json()
        .await
        .map_err(|error| WorkspaceSessionCreateError::Other(error.to_string()))?;
    Ok(payload.session.id)
}

#[cfg(not(target_family = "wasm"))]
pub(crate) async fn create_workspace_session(
    workspace_id: &str,
    checkout_ref: Option<String>,
) -> Result<String, WorkspaceSessionCreateError> {
    let _ =
        create_workspace_session_body(checkout_ref).map_err(WorkspaceSessionCreateError::Other)?;
    Err(WorkspaceSessionCreateError::Other(non_wasm_api_error(
        "POST",
        &workspace_sessions_url(workspace_id),
    )))
}

#[cfg(target_family = "wasm")]
pub(crate) async fn list_workspace_branches(
    workspace_id: &str,
) -> Result<Vec<WorkspaceBranch>, String> {
    let response = Request::get(&workspace_branches_url(workspace_id))
        .send()
        .await
        .map_err(|error| error.to_string())?;

    if !response.ok() {
        return Err(response_error_message(response, "List workspace branches failed").await);
    }

    let listed: WorkspaceBranchListResponse =
        response.json().await.map_err(|error| error.to_string())?;
    Ok(listed.branches)
}

#[cfg(not(target_family = "wasm"))]
pub(crate) async fn list_workspace_branches(
    workspace_id: &str,
) -> Result<Vec<WorkspaceBranch>, String> {
    Err(non_wasm_api_error("GET", &workspace_branches_url(workspace_id)))
}

#[cfg(target_family = "wasm")]
pub(crate) async fn list_workspace_sessions(
    workspace_id: &str,
) -> Result<Vec<SessionListItem>, String> {
    let response = Request::get(&workspace_sessions_url(workspace_id))
        .send()
        .await
        .map_err(|error| error.to_string())?;

    if !response.ok() {
        return Err(response_error_message(response, "List workspace sessions failed").await);
    }

    let listed: SessionListResponse = response.json().await.map_err(|error| error.to_string())?;
    Ok(listed.sessions)
}

#[cfg(not(target_family = "wasm"))]
pub(crate) async fn list_workspace_sessions(
    workspace_id: &str,
) -> Result<Vec<SessionListItem>, String> {
    Err(non_wasm_api_error(
        "GET",
        &workspace_sessions_url(workspace_id),
    ))
}

fn create_workspace_body(name: &str, upstream_url: String) -> Result<String, String> {
    serde_json::to_string(&CreateWorkspaceRequest {
        name: name.to_string(),
        upstream_url: normalize_required_text(upstream_url)?,
        credential_reference_id: None,
    })
    .map_err(|error| error.to_string())
}

fn create_workspace_session_body(checkout_ref: Option<String>) -> Result<String, String> {
    serde_json::to_string(&CreateSessionRequest {
        checkout_ref: normalize_optional_text(checkout_ref),
    })
    .map_err(|error| error.to_string())
}

fn update_workspace_body(name: Option<String>) -> Result<String, String> {
    serde_json::to_string(&UpdateWorkspaceRequest {
        name,
    })
    .map_err(|error| error.to_string())
}

fn workspace_url(workspace_id: &str) -> String {
    format!("{WORKSPACES_URL}/{}", encode_component(workspace_id))
}

fn workspace_sessions_url(workspace_id: &str) -> String {
    format!("{}/sessions", workspace_url(workspace_id))
}

fn workspace_branches_url(workspace_id: &str) -> String {
    format!("{}/branches", workspace_url(workspace_id))
}

fn normalize_optional_text(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let value = value.trim().to_string();
        (!value.is_empty()).then_some(value)
    })
}

fn normalize_required_text(value: String) -> Result<String, String> {
    let value = value.trim().to_string();
    if value.is_empty() {
        return Err("repository_url is required".to_string());
    }
    Ok(value)
}

#[cfg(not(target_family = "wasm"))]
fn non_wasm_api_error(method: &str, url: &str) -> String {
    format!("Browser {method} workspaces API is unavailable on non-wasm targets: {url}")
}

#[cfg(test)]
mod tests {
    use crate::infrastructure::api::poll_ready;

    use super::*;

    #[test]
    fn workspace_request_bodies_serialize_expected_payloads() {
        let body =
            create_workspace_body("My Workspace", " https://example.com/repo.git ".to_string())
                .expect("create body");
        assert!(body.contains("My Workspace"));
        assert!(body.contains("https://example.com/repo.git"));

        let body = update_workspace_body(Some("Renamed".to_string())).expect("update body");
        assert!(body.contains("Renamed"));

        let body =
            create_workspace_session_body(Some(" refs/heads/release ".to_string())).expect("body");
        assert!(body.contains("refs/heads/release"));
    }

    #[test]
    fn workspace_url_appends_workspace_id() {
        assert_eq!(workspace_url("w_123"), "/api/v1/workspaces/w_123");
        assert_eq!(workspace_url("w/1"), "/api/v1/workspaces/w%2F1");
        assert_eq!(
            workspace_sessions_url("w/1"),
            "/api/v1/workspaces/w%2F1/sessions"
        );
        assert_eq!(
            workspace_branches_url("w/1"),
            "/api/v1/workspaces/w%2F1/branches"
        );
        assert_eq!(WORKSPACES_URL, "/api/v1/workspaces");
    }

    #[test]
    fn host_workspace_api_functions_fail_with_descriptive_errors() {
        let list_error = poll_ready(list_workspaces()).expect_err("host list should fail");
        assert!(list_error.contains(WORKSPACES_URL));

        let create_error = poll_ready(create_workspace(
            "test",
            "https://example.com/repo.git".to_string(),
        ))
        .expect_err("host create should fail");
        assert!(create_error.contains(WORKSPACES_URL));

        let update_error = poll_ready(update_workspace("w_1", Some("new".to_string())))
            .expect_err("host update should fail");
        assert!(update_error.contains("/api/v1/workspaces/w_1"));

        let delete_error =
            poll_ready(delete_workspace("w_1")).expect_err("host delete should fail");
        assert!(delete_error.contains("/api/v1/workspaces/w_1"));

        let create_session_error = poll_ready(create_workspace_session("w_1", None))
            .expect_err("host workspace session create should fail");
        assert!(
            create_session_error
                .to_string()
                .contains("/api/v1/workspaces/w_1/sessions")
        );

        let list_branches_error = poll_ready(list_workspace_branches("w_1"))
            .expect_err("host workspace branches should fail");
        assert!(list_branches_error.contains("/api/v1/workspaces/w_1/branches"));

        let list_sessions_error = poll_ready(list_workspace_sessions("w_1"))
            .expect_err("host workspace session list should fail");
        assert!(list_sessions_error.contains("/api/v1/workspaces/w_1/sessions"));
    }

    #[test]
    fn workspace_session_create_errors_preserve_messages() {
        assert_eq!(
            WorkspaceSessionCreateError::NotFound("missing".to_string()).into_message(),
            "missing"
        );
        assert_eq!(
            WorkspaceSessionCreateError::Other("boom".to_string()).into_message(),
            "boom"
        );
    }

    #[test]
    fn normalize_optional_text_trims_and_drops_blank_values() {
        assert_eq!(
            normalize_optional_text(Some(" value ".to_string())),
            Some("value".to_string())
        );
        assert_eq!(normalize_optional_text(Some("   ".to_string())), None);
        assert_eq!(normalize_optional_text(None), None);
    }

    #[test]
    fn normalize_required_text_trims_and_rejects_blank_values() {
        assert_eq!(
            normalize_required_text(" value ".to_string()).expect("value should trim"),
            "value"
        );
        assert_eq!(
            normalize_required_text("   ".to_string()).expect_err("blank values should fail"),
            "repository_url is required"
        );
    }
}
