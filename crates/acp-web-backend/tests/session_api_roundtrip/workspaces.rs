use super::support::*;
use acp_web_backend::contract_workspaces::{CreateWorkspaceRequest, UpdateWorkspaceRequest};

#[tokio::test]
async fn workspace_crud_and_workspace_scoped_sessions_work_over_http() -> Result<()> {
    let stack = TestStack::spawn(ServerConfig {
        session_cap: 8,
        acp_server: String::new(),
        startup_hints: false,
        state_dir: test_state_dir(),
        frontend_dist: None,
    })
    .await?;

    let initial = stack.list_workspaces("alice").await?;
    assert_eq!(initial.workspaces.len(), 1);
    assert_eq!(initial.workspaces[0].name, "Default workspace");

    let created = stack
        .create_workspace(
            "alice",
            &CreateWorkspaceRequest {
                name: "Repo".to_string(),
                upstream_url: Some("https://example.com/repo.git".to_string()),
                default_ref: Some("refs/heads/main".to_string()),
                credential_reference_id: None,
            },
        )
        .await?;
    let workspace_id = created.workspace.workspace_id.clone();

    let fetched = stack.get_workspace("alice", &workspace_id).await?;
    assert_eq!(fetched.workspace.name, "Repo");

    let updated = stack
        .update_workspace(
            "alice",
            &workspace_id,
            &UpdateWorkspaceRequest {
                name: Some("Renamed repo".to_string()),
                default_ref: Some("refs/heads/release".to_string()),
            },
        )
        .await?;
    assert_eq!(updated.workspace.name, "Renamed repo");
    assert_eq!(
        updated.workspace.default_ref.as_deref(),
        Some("refs/heads/release")
    );

    let first = stack
        .create_workspace_session("alice", &workspace_id)
        .await?;
    let bootstrap_workspace_id = initial.workspaces[0].workspace_id.clone();
    let _legacy = stack.create_legacy_session("alice").await?;

    let listed = stack
        .list_workspace_sessions("alice", &workspace_id)
        .await?;
    assert_eq!(listed.sessions.len(), 1);
    assert_eq!(listed.sessions[0].id, first.session.id);
    assert_eq!(listed.sessions[0].workspace_id, workspace_id);

    let bootstrap_list = stack
        .list_workspace_sessions("alice", &bootstrap_workspace_id)
        .await?;
    assert!(
        bootstrap_list
            .sessions
            .iter()
            .all(|session| session.id != first.session.id)
    );

    stack.delete_session("alice", &first.session.id).await?;
    stack.delete_workspace("alice", &workspace_id).await?;
    let response = stack
        .client
        .get(format!(
            "{}/api/v1/workspaces/{workspace_id}",
            stack.backend_url
        ))
        .bearer_auth("alice")
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    Ok(())
}
