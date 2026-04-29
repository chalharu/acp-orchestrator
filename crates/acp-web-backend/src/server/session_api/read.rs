use axum::{
    Json,
    extract::{Extension, Path, Query, State},
};

use crate::contract_sessions::{
    SessionHistoryResponse, SessionListResponse, SessionResponse, SessionSnapshot, SessionStatus,
};
use crate::contract_slash::SlashCompletionsResponse;
use crate::{
    agent_runtime::{AgentRuntimeError, launch_session_blocking},
    auth::AuthenticatedPrincipal,
    completions::resolve_slash_completions,
    sessions::SessionStoreError,
    workspace_checkout::PreparedWorkspaceCheckout,
    workspace_records::{DurableSessionSnapshotRecord, SessionMetadataRecord},
};

use super::super::{AppError, AppState, OwnerContext, assets::SlashCompletionsQuery};

pub(in crate::server) async fn list_sessions(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
) -> Result<Json<SessionListResponse>, AppError> {
    let owner = state.owner_context(principal).await?;
    let sessions = state.store.list_owned_sessions(&owner.live_owner_id).await;

    Ok(Json(SessionListResponse { sessions }))
}

pub(in crate::server) async fn get_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
) -> Result<Json<SessionResponse>, AppError> {
    let owner = state.owner_context(principal).await?;
    let session = load_or_restore_session(&state, &owner, &session_id).await?;

    Ok(Json(SessionResponse { session }))
}

pub(in crate::server) async fn get_session_history(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
) -> Result<Json<SessionHistoryResponse>, AppError> {
    let owner = state.owner_context(principal).await?;
    let messages = load_or_restore_session(&state, &owner, &session_id)
        .await?
        .messages;

    Ok(Json(SessionHistoryResponse {
        session_id,
        messages,
    }))
}

pub(in crate::server) async fn get_slash_completions(
    State(state): State<AppState>,
    Query(query): Query<SlashCompletionsQuery>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
) -> Result<Json<SlashCompletionsResponse>, AppError> {
    let owner = state.owner_context(principal).await?;
    let response = resolve_slash_completions(
        &state.store,
        &owner.live_owner_id,
        &query.session_id,
        &query.prefix,
    )
    .await?;

    Ok(Json(response))
}

async fn load_or_restore_session(
    state: &AppState,
    owner: &OwnerContext,
    session_id: &str,
) -> Result<SessionSnapshot, AppError> {
    match state
        .store
        .session_snapshot(&owner.live_owner_id, session_id)
        .await
    {
        Ok(session) => Ok(session),
        Err(SessionStoreError::NotFound) => restore_durable_session(state, owner, session_id).await,
        Err(error) => Err(error.into()),
    }
}

async fn restore_durable_session(
    state: &AppState,
    owner: &OwnerContext,
    session_id: &str,
) -> Result<SessionSnapshot, AppError> {
    let (metadata, durable) = load_restorable_session(state, owner, session_id).await?;
    let restored = restore_durable_snapshot(state, owner, durable).await?;
    bind_restored_session_to_checkout(state, &owner.live_owner_id, &restored, metadata.as_ref())
        .await?;
    Ok(restored)
}

async fn load_restorable_session(
    state: &AppState,
    owner: &OwnerContext,
    session_id: &str,
) -> Result<(Option<SessionMetadataRecord>, DurableSessionSnapshotRecord), AppError> {
    let metadata = state
        .workspace_repository
        .load_session_metadata(&owner.user.user_id, session_id)
        .await?;
    let durable = state
        .workspace_repository
        .load_session_snapshot(&owner.user.user_id, session_id)
        .await?
        .ok_or_else(|| AppError::NotFound("session not found".to_string()))?;
    Ok((metadata, durable))
}

async fn restore_durable_snapshot(
    state: &AppState,
    owner: &OwnerContext,
    durable: DurableSessionSnapshotRecord,
) -> Result<SessionSnapshot, AppError> {
    let DurableSessionSnapshotRecord {
        session,
        last_activity_at,
    } = durable;
    state
        .store
        .restore_session(&owner.live_owner_id, session, last_activity_at)
        .await
        .map_err(AppError::from)
}

async fn bind_restored_session_to_checkout(
    state: &AppState,
    owner_id: &str,
    restored: &SessionSnapshot,
    metadata: Option<&SessionMetadataRecord>,
) -> Result<(), AppError> {
    let Some(checkout_relpath) = metadata.and_then(|record| record.checkout_relpath.as_deref())
    else {
        mark_restored_runtime_unavailable(
            state,
            owner_id,
            restored,
            "persisted checkout path is missing".to_string(),
        )
        .await?;
        return Ok(());
    };
    let Some(checkout) =
        resolve_restored_checkout(state, owner_id, restored, checkout_relpath, metadata).await?
    else {
        return Ok(());
    };
    if !launch_restored_agent_runtime(state, owner_id, restored, &checkout).await? {
        return Ok(());
    }
    match state
        .reply_provider
        .bind_session(&restored.id, checkout.working_dir.clone())
        .await
    {
        Ok(()) => Ok(()),
        Err(error) => {
            rollback_restored_session(state, owner_id, &restored.id, &error).await?;
            Err(AppError::Internal(error))
        }
    }
}

async fn resolve_restored_checkout(
    state: &AppState,
    owner_id: &str,
    restored: &SessionSnapshot,
    checkout_relpath: &str,
    metadata: Option<&SessionMetadataRecord>,
) -> Result<Option<PreparedWorkspaceCheckout>, AppError> {
    let Some(expected_relpath) = state
        .checkout_manager
        .checkout_relpath_for_session(&restored.id)
    else {
        mark_restored_runtime_unavailable(
            state,
            owner_id,
            restored,
            "checkout manager could not provide the restored session path".to_string(),
        )
        .await?;
        return Ok(None);
    };
    if checkout_relpath != expected_relpath {
        mark_restored_runtime_unavailable(
            state,
            owner_id,
            restored,
            "persisted checkout path does not match the restored session".to_string(),
        )
        .await?;
        return Ok(None);
    }
    match state
        .checkout_manager
        .resolve_checkout_path(checkout_relpath)
    {
        Some(path) => Ok(Some(PreparedWorkspaceCheckout {
            checkout_relpath: checkout_relpath.to_string(),
            checkout_ref: metadata.and_then(|record| record.checkout_ref.clone()),
            checkout_commit_sha: metadata.and_then(|record| record.checkout_commit_sha.clone()),
            working_dir: path,
        })),
        None => {
            let error = AppError::Internal("persisted checkout path is invalid".to_string());
            rollback_restored_session(state, owner_id, &restored.id, error.message()).await?;
            Err(error)
        }
    }
}

async fn launch_restored_agent_runtime(
    state: &AppState,
    owner_id: &str,
    restored: &SessionSnapshot,
    checkout: &PreparedWorkspaceCheckout,
) -> Result<bool, AppError> {
    if restored.status != SessionStatus::Active {
        return Ok(true);
    }

    let launch_result = launch_session_blocking(
        state.agent_runtime_manager.clone(),
        restored.id.clone(),
        restored.workspace_id.clone(),
        checkout.clone(),
    )
    .await;
    match launch_result {
        Ok(()) | Err(AgentRuntimeError::AlreadyRunning(_)) => Ok(true),
        Err(error) => {
            let message = error.to_string();
            mark_restored_runtime_unavailable(state, owner_id, restored, message).await?;
            Ok(false)
        }
    }
}

async fn mark_restored_runtime_unavailable(
    state: &AppState,
    owner_id: &str,
    restored: &SessionSnapshot,
    message: String,
) -> Result<(), AppError> {
    if restored.status != SessionStatus::Active {
        return Ok(());
    }
    state
        .store
        .mark_runtime_unavailable(owner_id, &restored.id, message.clone())
        .await?;
    tracing::warn!(
        session_id = %restored.id,
        workspace_id = %restored.workspace_id,
        "restored session runtime is unavailable: {message}"
    );
    Ok(())
}

async fn rollback_restored_session(
    state: &AppState,
    owner_id: &str,
    session_id: &str,
    error_message: &str,
) -> Result<(), AppError> {
    state.reply_provider.forget_session(session_id);
    state.agent_runtime_manager.forget_session(session_id);
    state
        .store
        .discard_session(owner_id, session_id)
        .await
        .map_err(|rollback_error| {
            AppError::Internal(format!(
                "{error_message}; restored session rollback failed: {}",
                rollback_error.message()
            ))
        })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{
        contract_sessions::SessionStatus,
        mock_client::{ReplyFuture, ReplyProvider, ReplyResult},
        sessions::SessionStore,
    };

    #[derive(Debug)]
    struct NoopReplyProvider;

    impl ReplyProvider for NoopReplyProvider {
        fn request_reply<'a>(&'a self, _turn: crate::sessions::TurnHandle) -> ReplyFuture<'a> {
            Box::pin(async { Ok(ReplyResult::NoOutput) })
        }
    }

    fn sample_session_snapshot(session_id: &str) -> SessionSnapshot {
        SessionSnapshot {
            id: session_id.to_string(),
            workspace_id: "w_test".to_string(),
            title: "Test session".to_string(),
            status: SessionStatus::Active,
            latest_sequence: 0,
            messages: Vec::new(),
            pending_permissions: Vec::new(),
            active_turn: false,
        }
    }

    fn sample_session_metadata(
        session_id: &str,
        checkout_relpath: Option<String>,
    ) -> SessionMetadataRecord {
        let now = chrono::Utc::now();
        SessionMetadataRecord {
            session_id: session_id.to_string(),
            workspace_id: "w_test".to_string(),
            owner_user_id: "alice".to_string(),
            title: "Test session".to_string(),
            status: "active".to_string(),
            checkout_relpath,
            checkout_ref: None,
            checkout_commit_sha: None,
            failure_reason: None,
            detach_deadline_at: None,
            restartable_deadline_at: None,
            created_at: now,
            last_activity_at: now,
            closed_at: None,
            deleted_at: None,
        }
    }

    async fn sample_turn_handle() -> crate::sessions::TurnHandle {
        let store = SessionStore::new(4);
        let session = store
            .create_session("alice", "w_test")
            .await
            .expect("session creation should succeed");
        store
            .submit_prompt("alice", &session.id, "hello".to_string())
            .await
            .expect("prompt submission should succeed")
            .turn_handle()
    }

    #[tokio::test]
    async fn noop_reply_provider_returns_no_output() {
        let reply = NoopReplyProvider
            .request_reply(sample_turn_handle().await)
            .await
            .expect("noop reply providers should return successfully");

        assert_eq!(reply, ReplyResult::NoOutput);
    }

    #[tokio::test]
    async fn restored_active_sessions_become_unavailable_without_checkout_metadata() {
        let store = Arc::new(SessionStore::new(4));
        let state = AppState::with_dependencies(store.clone(), Arc::new(NoopReplyProvider));
        let restored = sample_session_snapshot("s_restore");
        store
            .restore_session("alice", restored.clone(), chrono::Utc::now())
            .await
            .expect("restored session should enter live store");

        bind_restored_session_to_checkout(&state, "alice", &restored, None)
            .await
            .expect("missing checkout metadata should leave transcript readable");

        let error = store
            .submit_prompt("alice", &restored.id, "hello".to_string())
            .await
            .expect_err("runtime-unavailable restored sessions should reject writes");
        assert_eq!(error, SessionStoreError::RuntimeUnavailable);
    }

    #[tokio::test]
    async fn restored_active_sessions_become_unavailable_when_checkout_belongs_to_another_session()
    {
        let store = Arc::new(SessionStore::new(4));
        let state = AppState::with_dependencies(store.clone(), Arc::new(NoopReplyProvider));
        let restored = sample_session_snapshot("s_restore");
        let metadata =
            sample_session_metadata(&restored.id, Some("session-checkouts/s_other".to_string()));
        store
            .restore_session("alice", restored.clone(), chrono::Utc::now())
            .await
            .expect("restored session should enter live store");

        bind_restored_session_to_checkout(&state, "alice", &restored, Some(&metadata))
            .await
            .expect("mismatched checkout metadata should not block transcript reads");

        let error = store
            .submit_prompt("alice", &restored.id, "hello".to_string())
            .await
            .expect_err("runtime-unavailable restored sessions should reject writes");
        assert_eq!(error, SessionStoreError::RuntimeUnavailable);
    }

    #[tokio::test]
    async fn rollback_restored_session_reports_discard_failures_with_context() {
        let state = AppState::with_dependencies(
            Arc::new(SessionStore::new(4)),
            Arc::new(NoopReplyProvider),
        );

        let error =
            rollback_restored_session(&state, "alice", "missing", "binding checkout failed")
                .await
                .expect_err("missing restored sessions should surface rollback failures");

        assert!(matches!(error, AppError::Internal(message)
                if message == "binding checkout failed; restored session rollback failed: session not found"));
    }
}
