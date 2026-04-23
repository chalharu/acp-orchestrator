#![cfg_attr(not(target_family = "wasm"), allow(dead_code))]

use acp_contracts_workspaces::{WorkspaceListResponse, WorkspaceSummary};
#[cfg(target_family = "wasm")]
use gloo_net::http::Request;

#[cfg(target_family = "wasm")]
use super::response_error_message;

const WORKSPACES_URL: &str = "/api/v1/workspaces";

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
    Err(format!(
        "Browser GET workspaces API is unavailable on non-wasm targets: {WORKSPACES_URL}"
    ))
}

#[cfg(test)]
mod tests {
    use crate::infrastructure::api::poll_ready;

    use super::*;

    #[test]
    fn host_workspace_api_functions_fail_with_descriptive_errors() {
        let error = poll_ready(list_workspaces()).expect_err("host list should fail");
        assert!(error.contains(WORKSPACES_URL));
    }
}
