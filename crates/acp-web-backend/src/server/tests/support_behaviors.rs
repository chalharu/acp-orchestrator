use super::*;

#[test]
fn workspace_store_errors_map_to_the_expected_app_errors() {
    let unauthorized: AppError = WorkspaceStoreError::Unauthorized("auth".to_string()).into();
    let not_found: AppError = WorkspaceStoreError::NotFound("missing".to_string()).into();
    let conflict: AppError = WorkspaceStoreError::Conflict("conflict".to_string()).into();
    let bad_request: AppError = WorkspaceStoreError::Validation("invalid".to_string()).into();

    assert!(matches!(
        unauthorized,
        AppError::Unauthorized(message) if message == "auth"
    ));
    assert!(matches!(
        not_found,
        AppError::NotFound(message) if message == "missing"
    ));
    assert!(matches!(
        conflict,
        AppError::Conflict(message) if message == "conflict"
    ));
    assert!(matches!(
        bad_request,
        AppError::BadRequest(message) if message == "invalid"
    ));
}

#[tokio::test]
async fn tracking_reply_provider_returns_no_output() {
    let store = SessionStore::new(4);
    let session = store
        .create_session("alice")
        .await
        .expect("session creation should succeed");
    let pending = store
        .submit_prompt("alice", &session.id, "hello".to_string())
        .await
        .expect("prompt submission should succeed");
    let provider = TrackingReplyProvider {
        forgotten_sessions: StdArc::new(Mutex::new(Vec::new())),
    };

    let reply = provider
        .request_reply(pending.turn_handle())
        .await
        .expect("tracking providers should return cleanly");

    assert_eq!(reply, ReplyResult::NoOutput);
}

#[tokio::test]
async fn slash_completion_handler_returns_catalog_entries_for_the_owner() {
    let store = Arc::new(SessionStore::new(4));
    let state = AppState::with_dependencies(
        store.clone(),
        Arc::new(TrackingReplyProvider {
            forgotten_sessions: StdArc::new(Mutex::new(Vec::new())),
        }),
    );
    let session = store
        .create_session("alice")
        .await
        .expect("session creation should succeed");
    let response = get_slash_completions(
        State(state),
        Query(SlashCompletionsQuery {
            session_id: session.id,
            prefix: "/he".to_string(),
        }),
        bearer_principal("alice"),
    )
    .await
    .expect("authorized slash completion queries should succeed");

    assert_eq!(response.0.candidates.len(), 1);
    assert_eq!(response.0.candidates[0].label, "/help");
}

#[tokio::test]
async fn slash_completion_handler_requires_bearer_authentication() {
    let store = Arc::new(SessionStore::new(4));
    let state = AppState::with_dependencies(
        store.clone(),
        Arc::new(TrackingReplyProvider {
            forgotten_sessions: StdArc::new(Mutex::new(Vec::new())),
        }),
    );
    let session = store
        .create_session("alice")
        .await
        .expect("session creation should succeed");

    let response = app(state)
        .oneshot(
            axum::http::Request::builder()
                .uri(format!(
                    "/api/v1/completions/slash?sessionId={}&prefix=%2F",
                    session.id
                ))
                .body(Body::empty())
                .expect("request building should succeed"),
        )
        .await
        .expect("router calls should complete");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
