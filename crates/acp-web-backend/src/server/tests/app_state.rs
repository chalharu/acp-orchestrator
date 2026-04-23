use super::*;

#[test]
fn app_state_build_errors_format_and_expose_sources() {
    let reply_error = AppStateBuildError::from(MockClientError::TurnRuntime {
        message: "reply provider failed".to_string(),
    });
    let workspace_error = AppStateBuildError::from(WorkspaceStoreError::Database(
        "workspace store failed".to_string(),
    ));

    assert_eq!(
        reply_error.to_string(),
        "coordinating the prompt turn failed: reply provider failed"
    );
    assert_eq!(
        std::error::Error::source(&reply_error)
            .expect("reply provider sources should exist")
            .to_string(),
        "coordinating the prompt turn failed: reply provider failed"
    );
    assert_eq!(workspace_error.to_string(), "workspace store failed");
    assert_eq!(
        std::error::Error::source(&workspace_error)
            .expect("workspace store sources should exist")
            .to_string(),
        "workspace store failed"
    );
}

#[test]
fn app_state_debug_reports_public_fields() {
    let state = AppState::with_workspace_repository(
        Arc::new(SessionStore::new(4)),
        metadata_test_workspace_store(),
        Arc::new(TrackingReplyProvider {
            forgotten_sessions: StdArc::new(Mutex::new(Vec::new())),
        }),
    );
    let debug = format!("{state:?}");

    assert!(debug.contains("AppState"));
    assert!(debug.contains("startup_hints"));
    assert!(debug.contains("frontend_dist"));
}

#[test]
fn workspace_store_initialization_failures_map_into_app_state_build_errors() {
    let blocking_path = std::env::temp_dir().join(format!(
        "acp-web-backend-state-blocker-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let cleanup_path = blocking_path.clone();
    std::fs::write(&blocking_path, "blocker").expect("creating the blocking file should succeed");

    let error = SqliteWorkspaceRepository::new(blocking_path.join("db.sqlite"))
        .map_err(AppStateBuildError::from)
        .expect_err("invalid state roots should fail");

    assert!(matches!(error, AppStateBuildError::WorkspaceStore(_)));
    let _ = std::fs::remove_file(cleanup_path);
}

#[tokio::test]
async fn owner_context_surfaces_workspace_storage_failures() {
    let state = AppState::with_workspace_repository(
        Arc::new(SessionStore::new(4)),
        Arc::new(FailingWorkspaceStore::new("materialization unavailable")),
        Arc::new(TrackingReplyProvider {
            forgotten_sessions: StdArc::new(Mutex::new(Vec::new())),
        }),
    );
    let principal =
        authorize_request(&bearer_headers("alice"), true).expect("bearer headers should authorize");

    let error = state
        .owner_context(principal)
        .await
        .expect_err("owner context should surface workspace storage failures");
    assert!(matches!(error, AppError::Internal(message) if message == "internal server error"));
}

#[tokio::test]
async fn persist_session_metadata_best_effort_swallows_workspace_storage_errors() {
    let state = AppState::with_workspace_repository(
        Arc::new(SessionStore::new(4)),
        Arc::new(FailingWorkspaceStore::new("metadata unavailable")),
        Arc::new(TrackingReplyProvider {
            forgotten_sessions: StdArc::new(Mutex::new(Vec::new())),
        }),
    );

    persist_session_metadata_best_effort(
        &state,
        &sample_user_record(),
        &sample_snapshot("s_best_effort"),
        true,
        None,
        "test",
    )
    .await;
}

#[tokio::test]
async fn persist_prompt_snapshot_best_effort_swallows_snapshot_failures() {
    let state = AppState::with_workspace_repository(
        Arc::new(SessionStore::new(4)),
        metadata_test_workspace_store(),
        Arc::new(TrackingReplyProvider {
            forgotten_sessions: StdArc::new(Mutex::new(Vec::new())),
        }),
    );
    persist_prompt_snapshot_best_effort(
        &state,
        &sample_user_record(),
        "s_missing",
        Err(SessionStoreError::NotFound),
    )
    .await;
}

#[tokio::test]
async fn create_session_reports_metadata_rollback_failures() {
    let store = Arc::new(SessionStore::new(1));
    let state = AppState {
        store: store.clone(),
        workspace_repository: Arc::new(RollbackFailingMetadataWorkspaceStore::new(
            store,
            "alice",
            "metadata write failed",
            true,
        )),
        reply_provider: Arc::new(TrackingReplyProvider {
            forgotten_sessions: StdArc::new(Mutex::new(Vec::new())),
        }),
        startup_hints: false,
        frontend_dist: None,
    };

    let error = create_session(State(state), bearer_principal("alice"))
        .await
        .expect_err("metadata rollback failures should surface as internal errors");

    assert!(matches!(
        error,
        AppError::Internal(message)
            if message.contains("internal server error")
                && message.contains("session rollback failed: session not found")
    ));
}

#[tokio::test]
async fn create_session_rolls_back_when_metadata_persistence_fails() {
    let store = Arc::new(SessionStore::new(1));
    let state = AppState {
        store: store.clone(),
        workspace_repository: Arc::new(RollbackFailingMetadataWorkspaceStore::new(
            store.clone(),
            "alice",
            "metadata write failed",
            false,
        )),
        reply_provider: Arc::new(TrackingReplyProvider {
            forgotten_sessions: StdArc::new(Mutex::new(Vec::new())),
        }),
        startup_hints: false,
        frontend_dist: None,
    };

    let error = create_session(State(state), bearer_principal("alice"))
        .await
        .expect_err("metadata persistence failures should roll back the session");

    assert!(matches!(error, AppError::Internal(message) if message == "internal server error"));
    let snapshot_error = store
        .session_snapshot("alice", "s_1")
        .await
        .expect_err("failed creations should be rolled back");
    assert_eq!(snapshot_error, SessionStoreError::NotFound);
}

#[tokio::test]
async fn rename_session_keeps_working_when_workspace_materialization_fails() {
    let store = Arc::new(SessionStore::new(4));
    let session = store
        .create_session("alice", "w_test")
        .await
        .expect("session creation should succeed");
    let state = AppState::with_workspace_repository(
        store,
        Arc::new(FailingWorkspaceStore::new("metadata unavailable")),
        Arc::new(TrackingReplyProvider {
            forgotten_sessions: StdArc::new(Mutex::new(Vec::new())),
        }),
    );

    let renamed = rename_session(
        State(state),
        Path(session.id.clone()),
        bearer_principal("alice"),
        Json(RenameSessionRequest {
            title: "Renamed while metadata failed".to_string(),
        }),
    )
    .await
    .expect("live session rename should still succeed");

    assert_eq!(renamed.0.session.title, "Renamed while metadata failed");
}

#[tokio::test]
async fn post_message_keeps_working_when_workspace_materialization_fails() {
    let store = Arc::new(SessionStore::new(4));
    let session = store
        .create_session("alice", "w_test")
        .await
        .expect("session creation should succeed");
    let response = post_message(
        State(failing_workspace_state(store)),
        Path(session.id.clone()),
        bearer_principal("alice"),
        Json(PromptRequest {
            text: "hello from a degraded metadata store".to_string(),
        }),
    )
    .await
    .expect("live prompt submission should still succeed");

    assert!(response.0.accepted);
}

#[tokio::test]
async fn close_session_keeps_working_when_workspace_materialization_fails() {
    let store = Arc::new(SessionStore::new(4));
    let session = store
        .create_session("alice", "w_test")
        .await
        .expect("session creation should succeed");
    let response = close_session(
        State(failing_workspace_state(store)),
        Path(session.id.clone()),
        bearer_principal("alice"),
    )
    .await
    .expect("live session close should still succeed");

    assert_eq!(response.0.session.status, SessionStatus::Closed);
}

#[tokio::test]
async fn delete_session_keeps_working_when_workspace_materialization_fails() {
    let store = Arc::new(SessionStore::new(4));
    let session = store
        .create_session("alice", "w_test")
        .await
        .expect("session creation should succeed");
    let state = failing_workspace_state(store.clone());

    let response = delete_session(
        State(state),
        Path(session.id.clone()),
        bearer_principal("alice"),
    )
    .await
    .expect("live session deletion should still succeed");

    assert!(response.0.deleted);
    let snapshot_error = store
        .session_snapshot("alice", &session.id)
        .await
        .expect_err("deleted sessions should be removed from the live store");
    assert_eq!(snapshot_error, SessionStoreError::NotFound);
}
