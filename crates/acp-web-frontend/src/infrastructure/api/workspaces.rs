#![cfg_attr(not(target_family = "wasm"), allow(dead_code))]

use acp_contracts_workspaces::{
    CreateWorkspaceRequest, CreateWorkspaceResponse, DeleteWorkspaceResponse,
    UpdateWorkspaceRequest, UpdateWorkspaceResponse, WorkspaceDetail, WorkspaceListResponse,
    WorkspaceSummary,
};
use acp_contracts_sessions::CreateSessionResponse;
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
pub(crate) async fn create_workspace(name: &str) -> Result<WorkspaceDetail, String> {
    let body = create_workspace_body(name)?;
    let response = post_json_with_csrf(WORKSPACES_URL, body).await?;
    if !response.ok() {
        return Err(response_error_message(response, "Create workspace failed").await);
    }
    let payload: CreateWorkspaceResponse =
        response.json().await.map_err(|error| error.to_string())?;
    Ok(payload.workspace)
}

#[cfg(not(target_family = "wasm"))]
pub(crate) async fn create_workspace(name: &str) -> Result<WorkspaceDetail, String> {
    let _ = create_workspace_body(name)?;
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
) -> Result<String, WorkspaceSessionCreateError> {
    let response = post_json_with_csrf(&workspace_sessions_url(workspace_id), "{}".to_string())
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
) -> Result<String, WorkspaceSessionCreateError> {
    Err(WorkspaceSessionCreateError::Other(non_wasm_api_error(
        "POST",
        &workspace_sessions_url(workspace_id),
    )))
}

fn create_workspace_body(name: &str) -> Result<String, String> {
    serde_json::to_string(&CreateWorkspaceRequest {
        name: name.to_string(),
        upstream_url: None,
        default_ref: None,
        credential_reference_id: None,
    })
    .map_err(|error| error.to_string())
}

fn update_workspace_body(name: Option<String>) -> Result<String, String> {
    serde_json::to_string(&UpdateWorkspaceRequest {
        name,
        default_ref: None,
    })
    .map_err(|error| error.to_string())
}

fn workspace_url(workspace_id: &str) -> String {
    format!("{WORKSPACES_URL}/{}", encode_component(workspace_id))
}

fn workspace_sessions_url(workspace_id: &str) -> String {
    format!("{}/sessions", workspace_url(workspace_id))
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
        let body = create_workspace_body("My Workspace").expect("create body");
        assert!(body.contains("My Workspace"));

        let body = update_workspace_body(Some("Renamed".to_string())).expect("update body");
        assert!(body.contains("Renamed"));
    }

    #[test]
    fn workspace_url_appends_workspace_id() {
        assert_eq!(workspace_url("w_123"), "/api/v1/workspaces/w_123");
        assert_eq!(workspace_url("w/1"), "/api/v1/workspaces/w%2F1");
        assert_eq!(
            workspace_sessions_url("w/1"),
            "/api/v1/workspaces/w%2F1/sessions"
        );
        assert_eq!(WORKSPACES_URL, "/api/v1/workspaces");
    }

    #[test]
    fn host_workspace_api_functions_fail_with_descriptive_errors() {
        let list_error = poll_ready(list_workspaces()).expect_err("host list should fail");
        assert!(list_error.contains(WORKSPACES_URL));

        let create_error =
            poll_ready(create_workspace("test")).expect_err("host create should fail");
        assert!(create_error.contains(WORKSPACES_URL));

        let update_error = poll_ready(update_workspace("w_1", Some("new".to_string())))
            .expect_err("host update should fail");
        assert!(update_error.contains("/api/v1/workspaces/w_1"));

        let delete_error =
            poll_ready(delete_workspace("w_1")).expect_err("host delete should fail");
        assert!(delete_error.contains("/api/v1/workspaces/w_1"));

        let create_session_error = poll_ready(create_workspace_session("w_1"))
            .expect_err("host workspace session create should fail");
        assert!(create_session_error
            .to_string()
            .contains("/api/v1/workspaces/w_1/sessions"));
    }
}
