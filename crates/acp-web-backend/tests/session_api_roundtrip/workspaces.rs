use super::support::*;
use acp_web_backend::contract_workspaces::{CreateWorkspaceRequest, UpdateWorkspaceRequest};

async fn workspace_stack() -> Result<TestStack> {
    TestStack::spawn(ServerConfig {
        session_cap: 8,
        acp_server: String::new(),
        startup_hints: false,
        state_dir: test_state_dir(),
        frontend_dist: None,
    })
    .await
}

fn repo_workspace_request(name: &str) -> CreateWorkspaceRequest {
    CreateWorkspaceRequest {
        name: name.to_string(),
        upstream_url: Some("https://example.com/repo.git".to_string()),
        default_ref: Some("refs/heads/main".to_string()),
        credential_reference_id: None,
    }
}

async fn create_repo_workspace(stack: &TestStack, name: &str) -> Result<String> {
    Ok(stack
        .create_workspace("alice", &repo_workspace_request(name))
        .await?
        .workspace
        .workspace_id)
}

#[tokio::test]
async fn workspace_crud_works_over_http() -> Result<()> {
    let stack = workspace_stack().await?;

    let initial = stack.list_workspaces("alice").await?;
    assert_eq!(initial.workspaces.len(), 1);
    assert_eq!(initial.workspaces[0].name, "Default workspace");

    let workspace_id = create_repo_workspace(&stack, "Repo").await?;

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

#[tokio::test]
async fn workspace_sessions_are_scoped_over_http() -> Result<()> {
    let stack = workspace_stack().await?;
    let initial = stack.list_workspaces("alice").await?;
    let bootstrap_workspace_id = initial.workspaces[0].workspace_id.clone();
    let workspace_id = create_repo_workspace(&stack, "Repo").await?;
    let first = stack
        .create_workspace_session("alice", &workspace_id)
        .await?;
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
    Ok(())
}
