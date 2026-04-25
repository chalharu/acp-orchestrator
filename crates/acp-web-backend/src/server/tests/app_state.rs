use super::*;

#[derive(Debug)]
struct FailingCheckoutManager;

#[async_trait::async_trait]
impl crate::workspace_checkout::WorkspaceCheckoutManager for FailingCheckoutManager {
    async fn prepare_checkout(
        &self,
        _workspace: &WorkspaceRecord,
        _session_id: &str,
        _checkout_ref_override: Option<&str>,
    ) -> Result<
        crate::workspace_checkout::PreparedWorkspaceCheckout,
        crate::workspace_checkout::WorkspaceCheckoutError,
    > {
        Err(crate::workspace_checkout::WorkspaceCheckoutError::Git(
            "sensitive git detail".to_string(),
        ))
    }
}

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
        checkout_manager: test_checkout_manager(),
        startup_hints: false,
        frontend_dist: None,
    };

    let error = create_session(
        State(state),
        bearer_principal("alice"),
        axum::body::Bytes::new(),
    )
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
        checkout_manager: test_checkout_manager(),
        startup_hints: false,
        frontend_dist: None,
    };

    let error = create_session(
        State(state),
        bearer_principal("alice"),
        axum::body::Bytes::new(),
    )
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
async fn create_session_marks_provisioning_rows_failed_when_cloning_persistence_fails() {
    let store = Arc::new(SessionStore::new(1));
    let workspace_repository = Arc::new(
        RollbackFailingMetadataWorkspaceStore::with_save_failure_on_attempt(
            store.clone(),
            "alice",
            "cloning lifecycle failed",
            2,
        ),
    );
    let state = AppState {
        store: store.clone(),
        workspace_repository: workspace_repository.clone(),
        reply_provider: Arc::new(TrackingReplyProvider {
            forgotten_sessions: StdArc::new(Mutex::new(Vec::new())),
        }),
        checkout_manager: test_checkout_manager(),
        startup_hints: false,
        frontend_dist: None,
    };

    let error = create_session(
        State(state),
        bearer_principal("alice"),
        axum::body::Bytes::new(),
    )
    .await
    .expect_err("cloning lifecycle persistence failures should fail the request");

    assert!(matches!(error, AppError::Internal(message) if message == "internal server error"));
    let saved_metadata = workspace_repository
        .saved_metadata
        .lock()
        .expect("saved metadata should not poison")
        .clone();
    assert_eq!(saved_metadata.len(), 2);
    assert_eq!(saved_metadata[0].status, "provisioning");
    assert_eq!(saved_metadata[1].status, "failed");
    assert_eq!(saved_metadata[0].session_id, saved_metadata[1].session_id);
    let snapshot_error = store
        .session_snapshot("alice", &saved_metadata[0].session_id)
        .await
        .expect_err("failed creations should be rolled back");
    assert_eq!(snapshot_error, SessionStoreError::NotFound);
}

#[tokio::test]
async fn create_session_sanitizes_checkout_failures() {
    let state = AppState::with_workspace_repository_and_checkout_manager(
        Arc::new(SessionStore::new(4)),
        metadata_test_workspace_store(),
        Arc::new(TrackingReplyProvider {
            forgotten_sessions: StdArc::new(Mutex::new(Vec::new())),
        }),
        Arc::new(FailingCheckoutManager),
    );
    let workspace =
        create_owned_workspace_for_principal(&state, bearer_principal("alice"), "Workspace A")
            .await;

    let error = create_workspace_session(
        State(state),
        Path(workspace.workspace_id),
        bearer_principal("alice"),
        axum::body::Bytes::new(),
    )
    .await
    .expect_err("checkout failures should surface as internal errors");

    assert!(matches!(
        error,
        AppError::Internal(message) if message == "checkout preparation failed"
    ));
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
    let state = AppState::with_workspace_repository(
        store.clone(),
        Arc::new(FailingWorkspaceStore::new("metadata unavailable")),
        Arc::new(StaticReplyProvider {
            reply: "assistant reply despite metadata failure".to_string(),
        }),
    );
    let response = post_message(
        State(state),
        Path(session.id.clone()),
        bearer_principal("alice"),
        Json(PromptRequest {
            text: "hello from a degraded metadata store".to_string(),
        }),
    )
    .await
    .expect("live prompt submission should still succeed");

    assert!(response.0.accepted);
    timeout(std::time::Duration::from_secs(1), async {
        loop {
            let snapshot = store
                .session_snapshot("alice", &session.id)
                .await
                .expect("live session should stay accessible");
            if snapshot.messages.len() == 2 {
                assert_eq!(
                    snapshot.messages[1].text,
                    "assistant reply despite metadata failure"
                );
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("assistant completion should finish");
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
