use super::*;
use crate::contract_sessions::CreateSessionRequest;
use crate::contract_workspaces::{CreateWorkspaceRequest, UpdateWorkspaceRequest};

fn workspace_state() -> AppState {
    AppState::with_dependencies(
        Arc::new(SessionStore::new(4)),
        Arc::new(StaticReplyProvider {
            reply: String::new(),
        }),
    )
}

async fn create_owned_workspace(
    state: &AppState,
    name: &str,
) -> crate::contract_workspaces::WorkspaceDetail {
    create_workspace(
        State(state.clone()),
        bearer_principal("alice"),
        Json(CreateWorkspaceRequest {
            name: name.to_string(),
            upstream_url: "https://example.com/repo.git".to_string(),
            credential_reference_id: None,
        }),
    )
    .await
    .expect("workspace creation should succeed")
    .1
    .0
    .workspace
}

async fn create_workspace_session_for(
    state: &AppState,
    workspace_id: &str,
) -> crate::contract_sessions::SessionSnapshot {
    create_workspace_session(
        State(state.clone()),
        Path(workspace_id.to_string()),
        bearer_principal("alice"),
        axum::body::Bytes::new(),
    )
    .await
    .expect("workspace session should create")
    .1
    .0
    .session
}

#[tokio::test]
async fn workspace_session_routes_accept_empty_and_override_request_bodies() {
    let state = workspace_state();
    let workspace = create_owned_workspace(&state, "Compat").await;

    let empty_body = create_workspace_session(
        State(state.clone()),
        Path(workspace.workspace_id.clone()),
        bearer_principal("alice"),
        axum::body::Bytes::new(),
    )
    .await
    .expect("empty session body should remain supported");

    let override_body = create_workspace_session(
        State(state.clone()),
        Path(workspace.workspace_id.clone()),
        bearer_principal("alice"),
        axum::body::Bytes::from(
            serde_json::to_vec(&CreateSessionRequest {
                checkout_ref: Some("refs/heads/release".to_string()),
            })
            .expect("request should serialize"),
        ),
    )
    .await
    .expect("override session body should be accepted");

    assert_eq!(empty_body.0, StatusCode::CREATED);
    assert_eq!(override_body.0, StatusCode::CREATED);
}

#[tokio::test]
async fn listing_workspaces_starts_empty_until_a_workspace_is_created() {
    let state = workspace_state();

    let response = list_workspaces(State(state), bearer_principal("alice"))
        .await
        .expect("listing workspaces should succeed");

    assert!(response.0.workspaces.is_empty());
}

#[tokio::test]
async fn workspace_crud_handlers_round_trip_owned_workspaces() {
    let state = workspace_state();
    let created = create_owned_workspace(&state, "Repo").await;
    let workspace_id = created.workspace_id.clone();

    let fetched = get_workspace(
        State(state.clone()),
        Path(workspace_id.clone()),
        bearer_principal("alice"),
    )
    .await
    .expect("workspace lookup should succeed");
    assert_eq!(fetched.0.workspace.name, "Repo");
    assert_eq!(
        fetched.0.workspace.upstream_url.as_deref(),
        Some("https://example.com/repo.git")
    );

    let updated = update_workspace(
        State(state.clone()),
        Path(workspace_id.clone()),
        bearer_principal("alice"),
        Json(UpdateWorkspaceRequest {
            name: Some("Renamed repo".to_string()),
        }),
    )
    .await
    .expect("workspace update should succeed");
    assert_eq!(updated.0.workspace.name, "Renamed repo");

    let deleted = delete_workspace(
        State(state.clone()),
        Path(workspace_id.clone()),
        bearer_principal("alice"),
    )
    .await
    .expect("workspace delete should succeed");
    assert!(deleted.0.deleted);

    let error = get_workspace(State(state), Path(workspace_id), bearer_principal("alice"))
        .await
        .expect_err("deleted workspace should not load");
    assert!(matches!(error, AppError::NotFound(message) if message == "workspace not found"));
}

#[tokio::test]
async fn workspace_session_routes_scope_sessions_to_the_workspace() {
    let state = workspace_state();
    let first_workspace = create_owned_workspace(&state, "First").await;
    let second_workspace = create_owned_workspace(&state, "Second").await;
    let created = create_workspace_session_for(&state, &first_workspace.workspace_id).await;
    let other = create_workspace_session_for(&state, &second_workspace.workspace_id).await;

    assert_eq!(created.workspace_id, first_workspace.workspace_id);
    assert_eq!(other.workspace_id, second_workspace.workspace_id);

    let response = list_workspace_sessions(
        State(state),
        Path(first_workspace.workspace_id),
        bearer_principal("alice"),
    )
    .await
    .expect("listing workspace sessions should succeed");

    assert_eq!(response.0.sessions.len(), 1);
    assert_eq!(response.0.sessions[0].id, created.id);
    assert_eq!(response.0.sessions[0].workspace_id, created.workspace_id);
}

#[tokio::test]
async fn workspace_session_routes_list_durable_sessions_after_live_state_is_cleared() {
    let state = workspace_state();
    let workspace = create_owned_workspace(&state, "Durable").await;
    let created = create_workspace_session_for(&state, &workspace.workspace_id).await;

    state
        .store
        .delete_sessions_for_owners(&["alice".to_string()])
        .await;

    let response = list_workspace_sessions(
        State(state),
        Path(workspace.workspace_id),
        bearer_principal("alice"),
    )
    .await
    .expect("listing durable workspace sessions should succeed");

    assert_eq!(response.0.sessions.len(), 1);
    assert_eq!(response.0.sessions[0].id, created.id);
}

#[tokio::test]
async fn workspace_updates_require_name() {
    let state = workspace_state();
    let created = create_owned_workspace(&state, "Repo").await;

    let error = update_workspace(
        State(state),
        Path(created.workspace_id),
        bearer_principal("alice"),
        Json(UpdateWorkspaceRequest { name: None }),
    )
    .await
    .expect_err("workspace updates without mutable fields should fail");

    assert!(
        matches!(error, AppError::BadRequest(message) if message == "workspace update must include name")
    );
}

#[tokio::test]
async fn workspace_branch_routes_list_available_branches() {
    let state = workspace_state();
    let created = create_owned_workspace(&state, "Repo").await;

    let response = list_workspace_branches(
        State(state),
        Path(created.workspace_id),
        bearer_principal("alice"),
    )
    .await
    .expect("workspace branches should load");

    assert_eq!(
        response
            .0
            .branches
            .iter()
            .map(|branch| branch.ref_name.as_str())
            .collect::<Vec<_>>(),
        vec!["refs/heads/main", "refs/heads/release"]
    );
}
