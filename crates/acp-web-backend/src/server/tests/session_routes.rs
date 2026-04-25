use super::*;

fn state_with_static_reply(reply: &str) -> (Arc<SessionStore>, AppState) {
    let store = Arc::new(SessionStore::new(4));
    let state = AppState::with_dependencies(
        store.clone(),
        Arc::new(StaticReplyProvider {
            reply: reply.to_string(),
        }),
    );
    (store, state)
}

async fn create_persisted_workspace_session(
    state: &AppState,
    principal: Extension<AuthenticatedPrincipal>,
) -> crate::contract_sessions::SessionSnapshot {
    let workspace =
        create_owned_workspace_for_principal(state, principal.clone(), "Workspace A").await;
    create_workspace_session(
        State(state.clone()),
        Path(workspace.workspace_id),
        principal,
    )
    .await
    .expect("workspace session should create")
    .1
    .0
    .session
}

async fn wait_for_durable_messages(
    state: &AppState,
    user_id: &str,
    session_id: &str,
) -> crate::workspace_records::DurableSessionSnapshotRecord {
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let snapshot = state
                .workspace_repository
                .load_session_snapshot(user_id, session_id)
                .await
                .expect("durable snapshot should load");
            if let Some(snapshot) = snapshot
                && snapshot.session.messages.len() == 2
            {
                return snapshot;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("assistant reply should be durably persisted")
}

#[tokio::test]
async fn injected_reply_provider_handles_prompt_dispatch() {
    let (store, state) = state_with_static_reply("injected reply");
    let session = store
        .create_session("alice", "w_test")
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
async fn get_session_restores_durable_messages_after_live_session_is_cleared() {
    let (store, state) = state_with_static_reply("injected reply");
    let principal = bearer_principal("alice");
    let created = create_persisted_workspace_session(&state, principal.clone()).await;
    let _ = post_message(
        State(state.clone()),
        Path(created.id.clone()),
        principal.clone(),
        Json(PromptRequest {
            text: "hello".to_string(),
        }),
    )
    .await
    .expect("prompt submission should succeed");
    let durable_user = state
        .workspace_repository
        .materialize_user(&principal.0)
        .await
        .expect("principal materialization should succeed");

    let durable = wait_for_durable_messages(&state, &durable_user.user_id, &created.id).await;
    assert_eq!(durable.session.messages[1].text, "injected reply");

    store
        .delete_sessions_for_owners(&["alice".to_string()])
        .await;

    let restored = get_session(State(state), Path(created.id), principal)
        .await
        .expect("durably persisted sessions should restore");

    assert_eq!(restored.0.session.messages.len(), 2);
    assert_eq!(restored.0.session.messages[0].text, "hello");
    assert_eq!(restored.0.session.messages[1].text, "injected reply");
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
    let _workspace =
        create_owned_workspace_for_principal(&state, bearer_principal("alice"), "Workspace A")
            .await;
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
    let _workspace =
        create_owned_workspace_for_principal(&state, bearer_principal("alice"), "Workspace A")
            .await;
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
    let _workspace =
        create_owned_workspace_for_principal(&state, bearer_principal("alice"), "Workspace A")
            .await;
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
    let _workspace =
        create_owned_workspace_for_principal(&state, bearer_principal("alice"), "Workspace A")
            .await;
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
        .create_session("alice", "w_test")
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
    let _workspace =
        create_owned_workspace_for_principal(&state, bearer_principal("alice"), "Workspace A")
            .await;
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
async fn create_session_creates_a_standard_workspace_when_none_exist() {
    let state = AppState::with_dependencies(
        Arc::new(SessionStore::new(4)),
        Arc::new(StaticReplyProvider {
            reply: String::new(),
        }),
    );

    let response = create_session(State(state.clone()), bearer_principal("alice"))
        .await
        .expect("root create should create a session when no workspaces exist");
    let listed = list_workspaces(State(state), bearer_principal("alice"))
        .await
        .expect("workspace list should succeed after legacy session creation");

    assert_eq!(response.0, StatusCode::CREATED);
    assert_eq!(listed.0.workspaces.len(), 1);
    assert_eq!(listed.0.workspaces[0].name, "Workspace");
    assert_eq!(
        response.1.0.session.workspace_id,
        listed.0.workspaces[0].workspace_id
    );
}

#[tokio::test]
async fn create_session_requires_explicit_workspace_selection_when_multiple_workspaces_exist() {
    let state = AppState::with_dependencies(
        Arc::new(SessionStore::new(4)),
        Arc::new(StaticReplyProvider {
            reply: String::new(),
        }),
    );
    let _first =
        create_owned_workspace_for_principal(&state, bearer_principal("alice"), "Workspace A")
            .await;
    let _second =
        create_owned_workspace_for_principal(&state, bearer_principal("alice"), "Workspace B")
            .await;

    let error = create_session(State(state), bearer_principal("alice"))
        .await
        .expect_err("root create should reject ambiguous workspace selection");

    assert!(matches!(
        error,
        AppError::Conflict(message) if message == "workspace selection required"
    ));
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
        .create_session("alice", "w_test")
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
