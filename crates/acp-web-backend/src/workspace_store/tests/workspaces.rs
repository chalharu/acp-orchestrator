use super::*;
use crate::workspace_repository::{NewWorkspace, WorkspaceRepository, WorkspaceUpdatePatch};
use chrono::Utc;

async fn materialized_user(repository: &SqliteWorkspaceRepository) -> UserRecord {
    repository
        .materialize_user(&bearer_principal("developer"))
        .await
        .expect("principal materialization should succeed")
}

fn workspace_request(name: &str) -> NewWorkspace {
    NewWorkspace {
        name: name.to_string(),
        upstream_url: Some("https://example.com/repo.git".to_string()),
        default_ref: Some("refs/heads/main".to_string()),
        credential_reference_id: None,
    }
}

async fn create_workspace_record(
    repository: &SqliteWorkspaceRepository,
    user: &UserRecord,
    name: &str,
) -> WorkspaceRecord {
    repository
        .create_workspace(&user.user_id, &workspace_request(name))
        .await
        .expect("workspace creation should succeed")
}

async fn persist_workspace_session(
    repository: &SqliteWorkspaceRepository,
    user: &UserRecord,
    workspace_id: &str,
    session_id: &str,
    status: SessionStatus,
    deletion_reason: Option<&str>,
) {
    repository
        .persist_session_snapshot(
            &user.user_id,
            &SessionSnapshot {
                id: session_id.to_string(),
                workspace_id: workspace_id.to_string(),
                title: session_id.to_string(),
                status,
                latest_sequence: 0,
                messages: Vec::new(),
                pending_permissions: Vec::new(),
            },
            deletion_reason.is_none(),
            deletion_reason,
        )
        .await
        .expect("session metadata should persist");
}

#[tokio::test]
async fn workspaces_can_be_created_and_listed() {
    let repository = test_repository();
    let user = materialized_user(&repository).await;
    let created = create_workspace_record(&repository, &user, "Repo").await;

    assert_eq!(created.name, "Repo");
    assert_eq!(
        created.upstream_url.as_deref(),
        Some("https://example.com/repo.git")
    );

    let listed = repository
        .list_workspaces(&user.user_id)
        .await
        .expect("workspace listing should succeed");

    assert!(
        listed
            .iter()
            .any(|workspace| workspace.workspace_id == created.workspace_id)
    );
}

#[tokio::test]
async fn workspaces_can_be_updated_and_deleted() {
    let repository = test_repository();
    let user = materialized_user(&repository).await;
    let created = create_workspace_record(&repository, &user, "Repo").await;
    let updated = repository
        .update_workspace(
            &user.user_id,
            &created.workspace_id,
            &WorkspaceUpdatePatch {
                name: Some("Renamed repo".to_string()),
                default_ref: Some("refs/heads/release".to_string()),
            },
        )
        .await
        .expect("workspace update should succeed");

    assert_eq!(updated.name, "Renamed repo");
    assert_eq!(updated.default_ref.as_deref(), Some("refs/heads/release"));

    repository
        .delete_workspace(&user.user_id, &created.workspace_id)
        .await
        .expect("workspace delete should succeed");
    let loaded = repository
        .load_workspace(&user.user_id, &created.workspace_id)
        .await
        .expect("deleted workspace lookup should succeed");
    assert!(loaded.is_none());
}

#[tokio::test]
async fn workspace_updates_require_a_mutable_field() {
    let repository = test_repository();
    let user = materialized_user(&repository).await;
    let created = create_workspace_record(&repository, &user, "Repo").await;

    let error = repository
        .update_workspace(
            &user.user_id,
            &created.workspace_id,
            &WorkspaceUpdatePatch {
                name: None,
                default_ref: None,
            },
        )
        .await
        .expect_err("empty workspace updates should fail");

    assert_eq!(
        error,
        WorkspaceStoreError::Validation(
            "workspace update must include name or default_ref".to_string()
        )
    );
}

#[tokio::test]
async fn listing_workspace_sessions_returns_non_deleted_session_metadata() {
    let repository = test_repository();
    let user = materialized_user(&repository).await;
    let workspace = create_workspace_record(&repository, &user, "Repo").await;

    persist_workspace_session(
        &repository,
        &user,
        &workspace.workspace_id,
        "s_active",
        SessionStatus::Active,
        None,
    )
    .await;
    persist_workspace_session(
        &repository,
        &user,
        &workspace.workspace_id,
        "s_deleted",
        SessionStatus::Closed,
        Some("deleted"),
    )
    .await;

    let listed = repository
        .list_workspace_sessions(&user.user_id, &workspace.workspace_id)
        .await
        .expect("workspace session listing should succeed");

    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, "s_active");
    assert_eq!(listed[0].workspace_id, workspace.workspace_id);
}

#[tokio::test]
async fn workspace_repository_trait_methods_cover_read_and_write_ops() {
    let repository = test_repository();
    let user = materialized_user(&repository).await;
    let bootstrap = WorkspaceRepository::bootstrap_workspace(&repository, &user.user_id)
        .await
        .expect("bootstrap should succeed");
    let created = WorkspaceRepository::create_workspace(
        &repository,
        &user.user_id,
        &workspace_request("Trait Repo"),
    )
    .await
    .expect("workspace creation should succeed");
    let loaded =
        WorkspaceRepository::load_workspace(&repository, &user.user_id, &created.workspace_id)
            .await
            .expect("workspace lookup should succeed")
            .expect("created workspace should exist");
    let updated = WorkspaceRepository::update_workspace(
        &repository,
        &user.user_id,
        &created.workspace_id,
        &WorkspaceUpdatePatch {
            name: Some("Trait rename".to_string()),
            default_ref: Some("refs/heads/release".to_string()),
        },
    )
    .await
    .expect("workspace update should succeed");
    let sessions = WorkspaceRepository::list_workspace_sessions(
        &repository,
        &user.user_id,
        &created.workspace_id,
    )
    .await
    .expect("workspace session listing should succeed");

    assert!(
        WorkspaceRepository::list_workspaces(&repository, &user.user_id)
            .await
            .expect("workspace listing should succeed")
            .iter()
            .any(|workspace| workspace.workspace_id == bootstrap.workspace_id)
    );
    assert_eq!(loaded.workspace_id, created.workspace_id);
    assert_eq!(updated.default_ref.as_deref(), Some("refs/heads/release"));
    assert!(sessions.is_empty());
}

#[tokio::test]
async fn workspace_repository_trait_methods_cover_delete_ops() {
    let repository = test_repository();
    let user = materialized_user(&repository).await;
    let created = WorkspaceRepository::create_workspace(
        &repository,
        &user.user_id,
        &workspace_request("Trait Repo"),
    )
    .await
    .expect("workspace creation should succeed");
    WorkspaceRepository::delete_workspace(&repository, &user.user_id, &created.workspace_id)
        .await
        .expect("workspace deletion should succeed");
    let loaded =
        WorkspaceRepository::load_workspace(&repository, &user.user_id, &created.workspace_id)
            .await
            .expect("workspace lookup should succeed");

    assert!(loaded.is_none());
}

#[tokio::test]
async fn updating_and_deleting_missing_workspaces_return_not_found() {
    let repository = test_repository();
    let user = materialized_user(&repository).await;

    let update_error = repository
        .update_workspace(
            &user.user_id,
            "w_missing",
            &WorkspaceUpdatePatch {
                name: Some("Rename".to_string()),
                default_ref: None,
            },
        )
        .await
        .expect_err("updating a missing workspace should fail");
    let delete_error = repository
        .delete_workspace(&user.user_id, "w_missing")
        .await
        .expect_err("deleting a missing workspace should fail");

    assert_eq!(
        update_error,
        WorkspaceStoreError::NotFound("workspace not found".to_string())
    );
    assert_eq!(
        delete_error,
        WorkspaceStoreError::NotFound("workspace not found".to_string())
    );
}

#[tokio::test]
async fn bootstrap_workspaces_follow_standard_mutability_rules() {
    let repository = test_repository();
    let user = materialized_user(&repository).await;
    let bootstrap = repository
        .bootstrap_workspace(&user.user_id)
        .await
        .expect("bootstrap should succeed");

    let updated = repository
        .update_workspace(
            &user.user_id,
            &bootstrap.workspace_id,
            &WorkspaceUpdatePatch {
                name: Some("Renamed".to_string()),
                default_ref: None,
            },
        )
        .await
        .expect("bootstrap rename should succeed");
    repository
        .delete_workspace(&user.user_id, &bootstrap.workspace_id)
        .await
        .expect("empty bootstrap workspace should delete");

    assert_eq!(updated.name, "Renamed");
    assert!(
        repository
            .load_workspace(&user.user_id, &bootstrap.workspace_id)
            .await
            .expect("workspace lookup should succeed")
            .is_none()
    );
}

#[tokio::test]
async fn deleting_non_empty_workspaces_is_rejected() {
    let repository = test_repository();
    let user = materialized_user(&repository).await;
    let workspace = create_workspace_record(&repository, &user, "Repo").await;
    persist_workspace_session(
        &repository,
        &user,
        &workspace.workspace_id,
        "s_busy",
        SessionStatus::Active,
        None,
    )
    .await;

    let error = repository
        .delete_workspace(&user.user_id, &workspace.workspace_id)
        .await
        .expect_err("non-empty workspaces should not delete");

    assert_eq!(
        error,
        WorkspaceStoreError::Conflict("workspace_not_empty".to_string())
    );
}

#[tokio::test]
async fn loading_session_snapshots_rejects_invalid_payload_columns() {
    let repository = test_repository();
    let user = materialized_user(&repository).await;
    let workspace = create_workspace_record(&repository, &user, "Repo").await;
    persist_workspace_session(
        &repository,
        &user,
        &workspace.workspace_id,
        "s_invalid_payload",
        SessionStatus::Active,
        None,
    )
    .await;

    let connection = repository
        .open_connection()
        .expect("test repository connection should open");
    connection
        .execute(
            "UPDATE sessions SET latest_sequence = -1 WHERE session_id = 's_invalid_payload'",
            [],
        )
        .expect("corrupt latest_sequence should update");
    let latest_sequence_error = repository
        .load_session_snapshot_sync(&user.user_id, "s_invalid_payload")
        .expect_err("negative latest_sequence should fail to load");
    assert!(matches!(
        latest_sequence_error,
        WorkspaceStoreError::Database(message)
            if message.contains("invalid latest_sequence for session s_invalid_payload")
    ));

    connection
        .execute(
            "UPDATE sessions SET latest_sequence = 0, messages_json = 'not-json' WHERE session_id = 's_invalid_payload'",
            [],
        )
        .expect("corrupt messages_json should update");
    let messages_error = repository
        .load_session_snapshot_sync(&user.user_id, "s_invalid_payload")
        .expect_err("invalid messages_json should fail to load");
    assert!(matches!(
        messages_error,
        WorkspaceStoreError::Database(message)
            if message.contains("invalid messages_json for session s_invalid_payload")
    ));
}

#[tokio::test]
async fn failed_sessions_are_hidden_from_user_facing_durable_queries() {
    let repository = test_repository();
    let user = materialized_user(&repository).await;
    let workspace = create_workspace_record(&repository, &user, "Repo").await;
    let now = Utc::now();

    repository
        .save_session_metadata(&SessionMetadataRecord {
            session_id: "s_failed".to_string(),
            workspace_id: workspace.workspace_id.clone(),
            owner_user_id: user.user_id.clone(),
            title: "Failed".to_string(),
            status: "failed".to_string(),
            checkout_relpath: Some("session-checkouts/s_failed".to_string()),
            checkout_ref: Some("refs/heads/main".to_string()),
            checkout_commit_sha: Some("deadbeef".to_string()),
            failure_reason: Some("startup failed".to_string()),
            detach_deadline_at: None,
            restartable_deadline_at: None,
            created_at: now,
            last_activity_at: now,
            closed_at: None,
            deleted_at: None,
        })
        .await
        .expect("failed metadata should save");

    let listed = repository
        .list_workspace_sessions(&user.user_id, &workspace.workspace_id)
        .await
        .expect("workspace listing should succeed");
    let loaded = repository
        .load_session_snapshot(&user.user_id, "s_failed")
        .await
        .expect("durable lookup should succeed");
    let metadata = repository
        .load_session_metadata(&user.user_id, "s_failed")
        .await
        .expect("metadata lookup should succeed");

    assert!(listed.is_empty());
    assert!(loaded.is_none());
    assert_eq!(
        metadata
            .expect("failed metadata should remain durable")
            .failure_reason
            .as_deref(),
        Some("startup failed")
    );
}

#[test]
fn workspace_name_validation_rejects_blank_and_long_values() {
    assert_eq!(
        validate_workspace_name(" Repo ").expect("names should trim"),
        "Repo"
    );
    assert_eq!(
        validate_workspace_name("   ").expect_err("blank names should fail"),
        WorkspaceStoreError::Validation("workspace name must not be empty".to_string())
    );
    assert_eq!(
        validate_workspace_name(&"a".repeat(121)).expect_err("long names should fail"),
        WorkspaceStoreError::Validation(
            "workspace name must not exceed 120 characters".to_string()
        )
    );
}

#[test]
fn workspace_upstream_urls_are_trimmed_and_validated() {
    assert_eq!(
        validate_workspace_upstream_url(None).expect("missing urls should pass"),
        None
    );
    assert_eq!(
        validate_workspace_upstream_url(Some(" https://example.com/repo.git "))
            .expect("https urls should trim"),
        Some("https://example.com/repo.git".to_string())
    );
    assert_eq!(
        validate_workspace_upstream_url(Some("   ")).expect_err("blank urls should fail"),
        WorkspaceStoreError::Validation("upstream_url must not be empty".to_string())
    );
    assert_eq!(
        validate_workspace_upstream_url(Some("not a url")).expect_err("invalid urls should fail"),
        WorkspaceStoreError::Validation("upstream_url must be a valid URL".to_string())
    );
    assert_eq!(
        validate_workspace_upstream_url(Some("http://example.com/repo.git"))
            .expect_err("non-https urls should fail"),
        WorkspaceStoreError::Validation("upstream_url must use https".to_string())
    );
    assert_eq!(
        validate_workspace_upstream_url(Some("https://user:pass@example.com/repo.git"))
            .expect_err("credentialed urls should fail"),
        WorkspaceStoreError::Validation("upstream_url must not embed credentials".to_string())
    );
}

#[test]
fn default_refs_and_credentials_are_trimmed_and_validated() {
    assert_eq!(
        validate_workspace_default_ref(Some(" refs/heads/main ")).expect("refs should trim"),
        Some("refs/heads/main".to_string())
    );
    assert_eq!(
        validate_workspace_default_ref(None).expect("missing refs should pass"),
        None
    );
    assert_eq!(
        validate_workspace_default_ref(Some("   ")).expect_err("blank refs should fail"),
        WorkspaceStoreError::Validation("default_ref must not be empty".to_string())
    );
    assert_eq!(
        validate_workspace_default_ref(Some("refs/heads/feature branch"))
            .expect_err("invalid refs should fail"),
        WorkspaceStoreError::Validation("default_ref is invalid".to_string())
    );
    assert_eq!(
        validate_workspace_default_ref(Some("-branch"))
            .expect_err("dash-prefixed refs should fail"),
        WorkspaceStoreError::Validation("default_ref is invalid".to_string())
    );
    assert_eq!(
        validate_credential_reference_id(Some(" credential-1 "))
            .expect("credential ids should trim"),
        Some("credential-1".to_string())
    );
    assert_eq!(
        validate_credential_reference_id(Some("   "))
            .expect_err("blank credential ids should fail"),
        WorkspaceStoreError::Validation("credential_reference_id must not be empty".to_string())
    );
}
