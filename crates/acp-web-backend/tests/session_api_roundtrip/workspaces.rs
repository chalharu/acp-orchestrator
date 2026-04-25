use super::support::*;
use acp_web_backend::contract_sessions::CreateSessionRequest;
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

fn local_workspace_request(name: &str) -> CreateWorkspaceRequest {
    CreateWorkspaceRequest {
        name: name.to_string(),
        upstream_url: None,
        default_ref: None,
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

async fn create_local_workspace(stack: &TestStack, name: &str) -> Result<String> {
    Ok(stack
        .create_workspace("alice", &local_workspace_request(name))
        .await?
        .workspace
        .workspace_id)
}

async fn assert_workspace_session_scope(
    stack: &TestStack,
    workspace_id: &str,
    expected_session_id: &str,
    unexpected_session_id: Option<&str>,
) -> Result<()> {
    let listed = stack.list_workspace_sessions("alice", workspace_id).await?;
    assert_eq!(listed.sessions.len(), 1);
    assert_eq!(listed.sessions[0].id, expected_session_id);
    assert_eq!(listed.sessions[0].workspace_id, workspace_id);
    if let Some(unexpected_session_id) = unexpected_session_id {
        assert!(
            listed
                .sessions
                .iter()
                .all(|session| session.id != unexpected_session_id)
        );
    }
    Ok(())
}

async fn delete_sessions_and_workspaces(
    stack: &TestStack,
    session_ids: &[&str],
    workspace_ids: &[&str],
) -> Result<()> {
    for session_id in session_ids {
        stack.delete_session("alice", session_id).await?;
    }
    for workspace_id in workspace_ids {
        stack.delete_workspace("alice", workspace_id).await?;
    }
    Ok(())
}

#[tokio::test]
async fn workspace_crud_works_over_http() -> Result<()> {
    let stack = workspace_stack().await?;

    let initial = stack.list_workspaces("alice").await?;
    assert!(initial.workspaces.is_empty());

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
    let first_workspace_id = create_local_workspace(&stack, "Repo").await?;
    let second_workspace_id = create_local_workspace(&stack, "Repo Two").await?;
    let first = stack
        .create_workspace_session("alice", &first_workspace_id)
        .await?;
    let second = stack
        .create_workspace_session("alice", &second_workspace_id)
        .await?;

    assert_workspace_session_scope(
        &stack,
        &first_workspace_id,
        &first.session.id,
        Some(&second.session.id),
    )
    .await?;
    assert_workspace_session_scope(
        &stack,
        &second_workspace_id,
        &second.session.id,
        Some(&first.session.id),
    )
    .await?;
    delete_sessions_and_workspaces(
        &stack,
        &[&first.session.id, &second.session.id],
        &[&first_workspace_id, &second_workspace_id],
    )
    .await?;
    Ok(())
}

#[tokio::test]
async fn workspace_session_creation_accepts_empty_and_override_bodies_over_http() -> Result<()> {
    let stack = workspace_stack().await?;
    let workspace_id = create_local_workspace(&stack, "Repo").await?;

    let empty = stack
        .create_workspace_session("alice", &workspace_id)
        .await?;
    let override_request = CreateSessionRequest {
        checkout_ref: Some("HEAD".to_string()),
    };
    let overridden = stack
        .create_workspace_session_with_request("alice", &workspace_id, Some(&override_request))
        .await?;

    assert_eq!(empty.session.workspace_id, workspace_id);
    assert_eq!(overridden.session.workspace_id, workspace_id);
    assert_ne!(empty.session.id, overridden.session.id);
    Ok(())
}
