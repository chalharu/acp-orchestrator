use std::{convert::Infallible, sync::Arc, time::Duration};

use axum::{
    Json,
    extract::{Extension, Path, Query, State},
    response::sse::{Event, KeepAlive, Sse},
};
use futures_util::{Stream, StreamExt, stream};
use tokio_stream::wrappers::BroadcastStream;

use acp_contracts::{
    CancelTurnResponse, CloseSessionResponse, CreateSessionResponse, DeleteSessionResponse,
    PromptRequest, PromptResponse, RenameSessionRequest, RenameSessionResponse,
    ResolvePermissionRequest, ResolvePermissionResponse, SessionHistoryResponse,
    SessionListResponse, SessionResponse, SessionSnapshot, SlashCompletionsResponse, StreamEvent,
};

use crate::{
    auth::AuthenticatedPrincipal,
    completions::resolve_slash_completions,
    mock_client::{ReplyProvider, ReplyResult},
    sessions::{PendingPrompt, SessionStoreError},
    workspace_records::UserRecord,
};

use super::{AppError, AppState, assets::SlashCompletionsQuery};

#[derive(Debug, Clone)]
pub(super) struct LiveSessionWriteContext {
    pub(super) principal: AuthenticatedPrincipal,
    pub(super) user: Option<UserRecord>,
}

pub(super) async fn list_sessions(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
) -> Result<Json<SessionListResponse>, AppError> {
    let owner = state.owner_context(principal).await?;
    let sessions = state.store.list_owned_sessions(&owner.principal.id).await;

    Ok(Json(SessionListResponse { sessions }))
}

pub(super) async fn create_session(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
) -> Result<(axum::http::StatusCode, Json<CreateSessionResponse>), AppError> {
    let owner = state.owner_context(principal).await?;
    let session = state.store.create_session(&owner.principal.id).await?;
    let session_id = session.id.clone();
    let session = match seed_startup_hint(&state, &owner.principal.id, session).await {
        Ok(session) => session,
        Err(error) => {
            if let Err(rollback_error) =
                rollback_failed_session(&state, &owner.principal.id, &session_id).await
            {
                return Err(AppError::Internal(format!(
                    "{}; session rollback failed: {}",
                    error.message(),
                    rollback_error.message()
                )));
            }
            return Err(error);
        }
    };
    if let Err(error) = persist_session_metadata(&state, &owner.user, &session, true, None).await {
        if let Err(rollback_error) =
            rollback_failed_session(&state, &owner.principal.id, &session_id).await
        {
            return Err(AppError::Internal(format!(
                "{}; session rollback failed: {}",
                error.message(),
                rollback_error.message()
            )));
        }
        return Err(error);
    }

    Ok((
        axum::http::StatusCode::CREATED,
        Json(CreateSessionResponse { session }),
    ))
}

pub(super) async fn get_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
) -> Result<Json<SessionResponse>, AppError> {
    let owner = state.owner_context(principal).await?;
    let session = state
        .store
        .session_snapshot(&owner.principal.id, &session_id)
        .await?;

    Ok(Json(SessionResponse { session }))
}

pub(super) async fn rename_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
    Json(request): Json<RenameSessionRequest>,
) -> Result<Json<RenameSessionResponse>, AppError> {
    let owner = live_session_write_context(&state, principal, "rename").await?;
    let title = request.title.trim().to_string();
    if title.is_empty() {
        return Err(AppError::BadRequest("title must not be empty".to_string()));
    }
    if title.chars().count() > 500 {
        return Err(AppError::BadRequest(
            "title must not exceed 500 characters".to_string(),
        ));
    }
    let session = state
        .store
        .rename_session(&owner.principal.id, &session_id, title)
        .await?;
    if let Some(user) = owner.user.as_ref() {
        persist_session_metadata_for_user_best_effort(
            &state, user, &session, false, None, "rename",
        )
        .await;
    }

    Ok(Json(RenameSessionResponse { session }))
}

pub(super) async fn delete_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
) -> Result<Json<DeleteSessionResponse>, AppError> {
    let owner = live_session_write_context(&state, principal, "delete").await?;
    let snapshot = state
        .store
        .session_snapshot(&owner.principal.id, &session_id)
        .await?;
    state
        .store
        .delete_session(&owner.principal.id, &session_id)
        .await?;
    state.reply_provider.forget_session(&session_id);
    if let Some(user) = owner.user.as_ref() {
        persist_session_metadata_for_user_best_effort(
            &state,
            user,
            &snapshot,
            false,
            Some("deleted"),
            "delete",
        )
        .await;
    }

    Ok(Json(DeleteSessionResponse { deleted: true }))
}

pub(super) async fn get_session_history(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
) -> Result<Json<SessionHistoryResponse>, AppError> {
    let owner = state.owner_context(principal).await?;
    let messages = state
        .store
        .session_history(&owner.principal.id, &session_id)
        .await?;

    Ok(Json(SessionHistoryResponse {
        session_id,
        messages,
    }))
}

pub(super) async fn post_message(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
    Json(request): Json<PromptRequest>,
) -> Result<Json<PromptResponse>, AppError> {
    let owner = live_session_write_context(&state, principal, "submit_prompt").await?;
    let pending = state
        .store
        .submit_prompt(&owner.principal.id, &session_id, request.text)
        .await?;
    let snapshot_result = state
        .store
        .session_snapshot(&owner.principal.id, &session_id)
        .await;
    if let Some(user) = owner.user.as_ref() {
        persist_prompt_snapshot_best_effort(&state, user, &session_id, snapshot_result).await;
    }
    dispatch_assistant_request(state.reply_provider.clone(), pending);

    Ok(Json(PromptResponse { accepted: true }))
}

pub(super) async fn close_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
) -> Result<Json<CloseSessionResponse>, AppError> {
    let owner = live_session_write_context(&state, principal, "close").await?;
    let session = state
        .store
        .close_session(&owner.principal.id, &session_id)
        .await?;
    state.reply_provider.forget_session(&session_id);
    if let Some(user) = owner.user.as_ref() {
        persist_session_metadata_for_user_best_effort(
            &state,
            user,
            &session,
            false,
            Some("closed"),
            "close",
        )
        .await;
    }

    Ok(Json(CloseSessionResponse { session }))
}

pub(super) async fn cancel_turn(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
) -> Result<Json<CancelTurnResponse>, AppError> {
    let owner = state.owner_context(principal).await?;
    let cancelled = state
        .store
        .cancel_active_turn(&owner.principal.id, &session_id)
        .await?;

    Ok(Json(CancelTurnResponse { cancelled }))
}

pub(super) async fn resolve_permission(
    State(state): State<AppState>,
    Path((session_id, request_id)): Path<(String, String)>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
    Json(request): Json<ResolvePermissionRequest>,
) -> Result<Json<ResolvePermissionResponse>, AppError> {
    let owner = state.owner_context(principal).await?;
    let resolution = state
        .store
        .resolve_permission(
            &owner.principal.id,
            &session_id,
            &request_id,
            request.decision,
        )
        .await?;

    Ok(Json(resolution))
}

pub(super) async fn get_slash_completions(
    State(state): State<AppState>,
    Query(query): Query<SlashCompletionsQuery>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
) -> Result<Json<SlashCompletionsResponse>, AppError> {
    let owner = state.owner_context(principal).await?;
    let response = resolve_slash_completions(
        &state.store,
        &owner.principal.id,
        &query.session_id,
        &query.prefix,
    )
    .await?;

    Ok(Json(response))
}

pub(super) async fn stream_session_events(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let owner = state.owner_context(principal).await?;
    let (snapshot, receiver) = state
        .store
        .session_events(&owner.principal.id, &session_id)
        .await?;

    let initial_event = stream::once(async move {
        Ok::<Event, Infallible>(to_sse_event(StreamEvent::snapshot(snapshot)))
    });
    let updates = BroadcastStream::new(receiver)
        .filter_map(|result| async move { result.ok().map(to_sse_event).map(Ok) });

    Ok(Sse::new(initial_event.chain(updates)).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    ))
}

async fn persist_session_metadata(
    state: &AppState,
    user: &UserRecord,
    snapshot: &SessionSnapshot,
    touch_activity: bool,
    status_override: Option<&str>,
) -> Result<(), AppError> {
    state
        .workspace_repository
        .persist_session_snapshot(&user.user_id, snapshot, touch_activity, status_override)
        .await
        .map_err(AppError::from)?;
    Ok(())
}

pub(super) async fn persist_session_metadata_best_effort(
    state: &AppState,
    user: &UserRecord,
    snapshot: &SessionSnapshot,
    touch_activity: bool,
    status_override: Option<&str>,
    action: &'static str,
) {
    if let Err(error) =
        persist_session_metadata(state, user, snapshot, touch_activity, status_override).await
    {
        let error_message = error.message();
        tracing::warn!(
            session_id = %snapshot.id,
            owner_user_id = %user.user_id,
            action,
            "failed to persist session metadata: {error_message}"
        );
    }
}

async fn persist_session_metadata_for_user_best_effort(
    state: &AppState,
    user: &UserRecord,
    snapshot: &SessionSnapshot,
    touch_activity: bool,
    status_override: Option<&str>,
    action: &'static str,
) {
    persist_session_metadata_best_effort(
        state,
        user,
        snapshot,
        touch_activity,
        status_override,
        action,
    )
    .await;
}

async fn materialize_user_best_effort(
    state: &AppState,
    principal: &AuthenticatedPrincipal,
    action: &'static str,
) -> Option<UserRecord> {
    match state.workspace_repository.materialize_user(principal).await {
        Ok(user) => Some(user),
        Err(error) => {
            let error = AppError::from(error);
            let error_message = error.message();
            tracing::warn!(
                %error_message,
                principal_kind = ?principal.kind,
                action,
                "failed to materialize durable user"
            );
            None
        }
    }
}

async fn live_session_write_context(
    state: &AppState,
    principal: AuthenticatedPrincipal,
    action: &'static str,
) -> Result<LiveSessionWriteContext, AppError> {
    match principal.kind {
        crate::auth::AuthenticatedPrincipalKind::Bearer => Ok(LiveSessionWriteContext {
            user: materialize_user_best_effort(state, &principal, action).await,
            principal,
        }),
        crate::auth::AuthenticatedPrincipalKind::BrowserSession => {
            let owner = state.owner_context(principal).await?;
            Ok(LiveSessionWriteContext {
                principal: owner.principal,
                user: Some(owner.user),
            })
        }
    }
}

pub(super) async fn persist_prompt_snapshot_best_effort(
    state: &AppState,
    user: &UserRecord,
    session_id: &str,
    snapshot_result: Result<SessionSnapshot, SessionStoreError>,
) {
    match snapshot_result {
        Ok(snapshot) => {
            persist_session_metadata_for_user_best_effort(
                state,
                user,
                &snapshot,
                true,
                None,
                "submit_prompt",
            )
            .await;
        }
        Err(error) => {
            let error_message = error.message();
            tracing::warn!(
                session_id = %session_id,
                "failed to snapshot session metadata after prompt submission: {error_message}"
            );
        }
    }
}

async fn seed_startup_hint(
    state: &AppState,
    owner: &str,
    session: SessionSnapshot,
) -> Result<SessionSnapshot, AppError> {
    if !state.startup_hints {
        return Ok(session);
    }

    let Some(hint) = state
        .reply_provider
        .prime_session(&session.id)
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?
    else {
        return Ok(session);
    };

    state
        .store
        .append_assistant_message(owner, &session.id, hint)
        .await
        .map_err(AppError::from)
}

async fn rollback_failed_session(
    state: &AppState,
    owner: &str,
    session_id: &str,
) -> Result<(), AppError> {
    state.reply_provider.forget_session(session_id);
    state
        .store
        .discard_session(owner, session_id)
        .await
        .map_err(|error| AppError::Internal(error.message().to_string()))
}

fn dispatch_assistant_request(reply_provider: Arc<dyn ReplyProvider>, pending: PendingPrompt) {
    tokio::spawn(async move {
        match reply_provider.request_reply(pending.turn_handle()).await {
            Ok(ReplyResult::Reply(reply)) => pending.complete_with_reply(reply).await,
            Ok(ReplyResult::Status(message)) => pending.complete_with_status(message).await,
            Ok(ReplyResult::NoOutput) => pending.complete_without_output().await,
            Err(error) => {
                pending
                    .complete_with_status(format!("ACP request failed: {error}"))
                    .await;
            }
        }
    });
}

fn to_sse_event(event: StreamEvent) -> Event {
    let sequence = event.sequence.to_string();
    let payload =
        serde_json::to_string(&event).expect("stream events should always serialize successfully");

    Event::default()
        .event(event.event_name())
        .id(sequence)
        .data(payload)
}
