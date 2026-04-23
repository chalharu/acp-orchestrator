use super::*;
use crate::contract_workspaces::{CreateWorkspaceRequest, UpdateWorkspaceRequest};

#[tokio::test]
async fn listing_workspaces_bootstraps_the_default_workspace() {
    let state = AppState::with_dependencies(
        Arc::new(SessionStore::new(4)),
        Arc::new(StaticReplyProvider {
            reply: String::new(),
        }),
    );

    let response = list_workspaces(State(state), bearer_principal("alice"))
        .await
        .expect("listing workspaces should succeed");

    assert_eq!(response.0.workspaces.len(), 1);
    assert_eq!(response.0.workspaces[0].name, "Default workspace");
    assert_eq!(
        response.0.workspaces[0].bootstrap_kind.as_deref(),
        Some("legacy-session-routes")
    );
}

#[tokio::test]
async fn workspace_crud_handlers_round_trip_owned_workspaces() {
    let state = AppState::with_dependencies(
        Arc::new(SessionStore::new(4)),
        Arc::new(StaticReplyProvider {
            reply: String::new(),
        }),
    );

    let created = create_workspace(
        State(state.clone()),
        bearer_principal("alice"),
        Json(CreateWorkspaceRequest {
            name: "Repo".to_string(),
            upstream_url: Some("https://example.com/repo.git".to_string()),
            default_ref: Some("refs/heads/main".to_string()),
            credential_reference_id: None,
        }),
    )
    .await
    .expect("workspace creation should succeed");
    let workspace_id = created.1.0.workspace.workspace_id.clone();

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
            default_ref: Some("refs/heads/release".to_string()),
        }),
    )
    .await
    .expect("workspace update should succeed");
    assert_eq!(updated.0.workspace.name, "Renamed repo");
    assert_eq!(
        updated.0.workspace.default_ref.as_deref(),
        Some("refs/heads/release")
    );

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
    let state = AppState::with_dependencies(
        Arc::new(SessionStore::new(4)),
        Arc::new(StaticReplyProvider {
            reply: String::new(),
        }),
    );

    let first_workspace = create_workspace(
        State(state.clone()),
        bearer_principal("alice"),
        Json(CreateWorkspaceRequest {
            name: "First".to_string(),
            upstream_url: None,
            default_ref: None,
            credential_reference_id: None,
        }),
    )
    .await
    .expect("first workspace should create")
    .1
    .0
    .workspace;
    let second_workspace = create_workspace(
        State(state.clone()),
        bearer_principal("alice"),
        Json(CreateWorkspaceRequest {
            name: "Second".to_string(),
            upstream_url: None,
            default_ref: None,
            credential_reference_id: None,
        }),
    )
    .await
    .expect("second workspace should create")
    .1
    .0
    .workspace;

    let created = create_workspace_session(
        State(state.clone()),
        Path(first_workspace.workspace_id.clone()),
        bearer_principal("alice"),
    )
    .await
    .expect("workspace session should create");
    let other = create_workspace_session(
        State(state.clone()),
        Path(second_workspace.workspace_id.clone()),
        bearer_principal("alice"),
    )
    .await
    .expect("other workspace session should create");

    assert_eq!(
        created.1.0.session.workspace_id,
        first_workspace.workspace_id
    );
    assert_eq!(
        other.1.0.session.workspace_id,
        second_workspace.workspace_id
    );

    let response = list_workspace_sessions(
        State(state),
        Path(first_workspace.workspace_id),
        bearer_principal("alice"),
    )
    .await
    .expect("listing workspace sessions should succeed");

    assert_eq!(response.0.sessions.len(), 1);
    assert_eq!(response.0.sessions[0].id, created.1.0.session.id);
    assert_eq!(
        response.0.sessions[0].workspace_id,
        created.1.0.session.workspace_id
    );
}

#[tokio::test]
async fn workspace_updates_require_name_or_default_ref() {
    let state = AppState::with_dependencies(
        Arc::new(SessionStore::new(4)),
        Arc::new(StaticReplyProvider {
            reply: String::new(),
        }),
    );
    let created = create_workspace(
        State(state.clone()),
        bearer_principal("alice"),
        Json(CreateWorkspaceRequest {
            name: "Repo".to_string(),
            upstream_url: Some("https://example.com/repo.git".to_string()),
            default_ref: None,
            credential_reference_id: None,
        }),
    )
    .await
    .expect("workspace creation should succeed")
    .1
    .0
    .workspace;

    let error = update_workspace(
        State(state),
        Path(created.workspace_id),
        bearer_principal("alice"),
        Json(UpdateWorkspaceRequest {
            name: None,
            default_ref: None,
        }),
    )
    .await
    .expect_err("workspace updates without mutable fields should fail");

    assert!(
        matches!(error, AppError::BadRequest(message) if message == "workspace update must include name or default_ref")
    );
}
