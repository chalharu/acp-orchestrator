use super::*;

#[tokio::test]
async fn injected_reply_provider_handles_prompt_dispatch() {
    let store = Arc::new(SessionStore::new(4));
    let state = AppState::with_dependencies(
        store.clone(),
        Arc::new(StaticReplyProvider {
            reply: "injected reply".to_string(),
        }),
    );
    let session = store
        .create_session("alice")
        .await
        .expect("session creation should succeed");
    let _ = post_message(
        State(state),
        Path(session.id.clone()),
        bearer_principal("alice"),
        Json(PromptRequest {
            text: "hello".to_string(),
        }),
    )
    .await
    .expect("prompt submission should succeed");

    let history = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let history = store
                .session_history("alice", &session.id)
                .await
                .expect("session history should load");
            if history.len() == 2 {
                return history;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("assistant reply should be recorded");

    assert_eq!(history[1].text, "injected reply");
}

#[tokio::test]
async fn create_session_seeds_startup_hints_when_enabled() {
    let store = Arc::new(SessionStore::new(4));
    let state = AppState {
        store: store.clone(),
        workspace_repository: new_ephemeral_workspace_repository(),
        reply_provider: Arc::new(StartupHintProvider {
            hint: "bundled mock verification ready".to_string(),
        }),
        startup_hints: true,
        frontend_dist: None,
    };
    let response = create_session(State(state), bearer_principal("alice"))
        .await
        .expect("session creation should succeed");

    assert_eq!(response.0, StatusCode::CREATED);
    assert_eq!(response.1.0.session.messages.len(), 1);
    assert_eq!(
        response.1.0.session.messages[0].text,
        "bundled mock verification ready"
    );
}

#[tokio::test]
async fn create_session_skips_startup_hints_when_disabled() {
    let store = Arc::new(SessionStore::new(4));
    let state = AppState {
        store,
        workspace_repository: new_ephemeral_workspace_repository(),
        reply_provider: Arc::new(StartupHintProvider {
            hint: "should stay hidden".to_string(),
        }),
        startup_hints: false,
        frontend_dist: None,
    };
    let response = create_session(State(state), bearer_principal("alice"))
        .await
        .expect("session creation should succeed");

    assert_eq!(response.0, StatusCode::CREATED);
    assert!(response.1.0.session.messages.is_empty());
}

#[tokio::test]
async fn create_session_keeps_sessions_without_primeable_startup_hints() {
    #[derive(Debug)]
    struct NoStartupHintProvider;

    impl ReplyProvider for NoStartupHintProvider {
        fn request_reply<'a>(&'a self, _turn: TurnHandle) -> ReplyFuture<'a> {
            Box::pin(async { Ok(ReplyResult::NoOutput) })
        }
    }

    let store = Arc::new(SessionStore::new(4));
    let state = AppState {
        store,
        workspace_repository: new_ephemeral_workspace_repository(),
        reply_provider: Arc::new(NoStartupHintProvider),
        startup_hints: true,
        frontend_dist: None,
    };
    let response = create_session(State(state), bearer_principal("alice"))
        .await
        .expect("session creation should succeed");

    assert_eq!(response.0, StatusCode::CREATED);
    assert!(response.1.0.session.messages.is_empty());
}

#[tokio::test]
async fn create_session_rolls_back_when_startup_hints_fail() {
    let store = Arc::new(SessionStore::new(1));
    let forgotten_sessions = StdArc::new(Mutex::new(Vec::new()));
    let state = AppState {
        store: store.clone(),
        workspace_repository: new_ephemeral_workspace_repository(),
        reply_provider: Arc::new(FailingStartupHintProvider {
            forgotten_sessions: forgotten_sessions.clone(),
        }),
        startup_hints: true,
        frontend_dist: None,
    };
    let error = create_session(State(state), bearer_principal("alice"))
        .await
        .expect_err("failed startup hint priming should fail the request");

    assert!(
        matches!(error, AppError::Internal(message) if message.contains("startup hint priming failed"))
    );
    let rolled_back_session_id = forgotten_sessions
        .lock()
        .expect("cleanup tracking should not poison")
        .first()
        .cloned()
        .expect("failed priming should forget the provisional session");
    let snapshot_error = store
        .session_snapshot("alice", &rolled_back_session_id)
        .await
        .expect_err("rolled back sessions should be removed");
    assert_eq!(snapshot_error, SessionStoreError::NotFound);
    store
        .create_session("alice")
        .await
        .expect("rollback should free the session cap");
}

#[tokio::test]
async fn create_session_reports_rollback_failures() {
    let store = Arc::new(SessionStore::new(1));
    let forgotten_sessions = StdArc::new(Mutex::new(Vec::new()));
    let state = AppState {
        store: store.clone(),
        workspace_repository: new_ephemeral_workspace_repository(),
        reply_provider: Arc::new(RollbackFailingStartupHintProvider {
            store: store.clone(),
            owner: "alice".to_string(),
            forgotten_sessions: forgotten_sessions.clone(),
        }),
        startup_hints: true,
        frontend_dist: None,
    };
    let error = create_session(State(state), bearer_principal("alice"))
        .await
        .expect_err("rollback failures should surface as internal errors");

    assert!(matches!(
        error,
        AppError::Internal(message)
            if message.contains("startup hint priming failed")
                && message.contains("session rollback failed: session not found")
    ));
    assert_eq!(
        forgotten_sessions
            .lock()
            .expect("cleanup tracking should not poison")
            .len(),
        1
    );
}

#[tokio::test]
async fn closing_sessions_notifies_reply_provider_cleanup() {
    let store = Arc::new(SessionStore::new(4));
    let forgotten_sessions = StdArc::new(Mutex::new(Vec::new()));
    let state = AppState::with_dependencies(
        store.clone(),
        Arc::new(TrackingReplyProvider {
            forgotten_sessions: forgotten_sessions.clone(),
        }),
    );
    let session = store
        .create_session("alice")
        .await
        .expect("session creation should succeed");
    let response = close_session(
        State(state),
        Path(session.id.clone()),
        bearer_principal("alice"),
    )
    .await
    .expect("closing the session should succeed");

    assert_eq!(response.0.session.id, session.id);
    assert_eq!(
        forgotten_sessions
            .lock()
            .expect("cleanup tracking should not poison")
            .as_slice(),
        [session.id]
    );
}

#[tokio::test]
async fn legacy_session_routes_persist_owner_scoped_metadata() {
    let context = metadata_test_context().await;
    assert_session_routes_persist_owner_scoped_metadata(&context).await;
}

#[tokio::test]
async fn browser_session_routes_persist_owner_scoped_metadata() {
    let context = browser_metadata_test_context().await;
    assert_session_routes_persist_owner_scoped_metadata(&context).await;
}

#[tokio::test]
async fn browser_session_writes_require_an_authenticated_account() {
    let context = browser_metadata_test_context().await;
    let (session, _) = create_persisted_session(&context).await;
    let replacement_admin = context
        .workspace_repository
        .create_local_account("backup-admin", "password123", true)
        .await
        .expect("creating a replacement admin should succeed");
    let invalidated_browser_sessions = context
        .workspace_repository
        .delete_local_account(&context.user.user_id, &replacement_admin.user_id)
        .await
        .expect("deleting the account should invalidate its browser sessions");

    assert_eq!(
        invalidated_browser_sessions,
        vec![context.live_owner_id.clone()]
    );

    let error = post_message(
        State(context.state.clone()),
        Path(session.id.clone()),
        context.principal.clone(),
        Json(PromptRequest {
            text: "should not run".to_string(),
        }),
    )
    .await
    .expect_err("deleted browser accounts should not post prompts");

    assert!(
        matches!(error, AppError::Unauthorized(message) if message == "account authentication required")
    );
    let snapshot = context
        .store
        .session_snapshot(&context.live_owner_id, &session.id)
        .await
        .expect("live session should still exist before in-memory invalidation");
    assert!(snapshot.messages.is_empty());
}

#[tokio::test]
async fn create_session_scrubs_workspace_store_failures() {
    let state = AppState::with_workspace_repository(
        Arc::new(SessionStore::new(4)),
        Arc::new(FailingWorkspaceStore::new("db path leaked")),
        Arc::new(TrackingReplyProvider {
            forgotten_sessions: StdArc::new(Mutex::new(Vec::new())),
        }),
    );

    let error = create_session(State(state), bearer_principal("alice"))
        .await
        .expect_err("workspace store failures should surface as internal errors");

    assert!(matches!(error, AppError::Internal(message) if message == "internal server error"));
}
