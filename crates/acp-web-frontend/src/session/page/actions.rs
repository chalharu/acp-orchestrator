use acp_contracts::{
    CompletionCandidate, ConversationMessage, MessageRole, PermissionDecision, PermissionRequest,
    SessionSnapshot, SessionStatus, StreamEvent, StreamEventPayload,
};
use futures_util::{
    StreamExt,
    future::{AbortHandle, Abortable},
};
use leptos::prelude::*;

use crate::application::auth::{self, HomeRouteTarget};
use crate::browser::{
    clear_draft, clear_prepared_session_id, clear_prepared_session_id_if_matches, navigate_to,
    prepared_session_id, store_prepared_session_id,
};
use crate::components::composer::ComposerSlashCallbacks;
use crate::domain::routing::app_session_path;
use crate::domain::session::{
    PendingPermission, SessionLifecycle, TurnState, mark_session_closed, message_to_entry,
    next_session_destination, remove_session_from_list, rename_session_in_list,
    session_action_busy, session_bootstrap_from_snapshot, session_end_message,
    should_apply_snapshot_turn_state, should_release_turn_state, tool_activity_text,
    turn_state_for_snapshot,
};
use crate::domain::transcript::{EntryRole, TranscriptEntry};
use crate::infrastructure::api;
use crate::slash::{
    BrowserSlashAction, apply_slash_completion, cycle_slash_selection, local_browser_commands,
    local_slash_candidates, parse_browser_slash_action,
};

use super::state::SessionSignals;

pub(super) fn spawn_home_redirect(error: RwSignal<Option<String>>, preparing: RwSignal<bool>) {
    leptos::task::spawn_local(async move {
        let result = match api::auth_status().await {
            Ok(status) => navigate_home_target(auth::home_route_target(&status)).await,
            Err(message) => Err(message),
        };

        if let Err(message) = result {
            set_home_redirect_error(error, preparing, message);
        }
    });
}

pub(super) fn spawn_session_bootstrap(session_id: String, signals: SessionSignals) {
    leptos::task::spawn_local(async move {
        match api::load_session(&session_id).await {
            Ok(session) => {
                let is_closed = session.status == SessionStatus::Closed;
                apply_loaded_session(session, signals);
                refresh_session_list(signals).await;
                if !is_closed {
                    spawn_session_stream(session_id.clone(), signals);
                }
            }
            Err(api::SessionLoadError::ResumeUnavailable(message)) => {
                record_session_bootstrap_failure(message, SessionLifecycle::Unavailable, signals);
                refresh_session_list(signals).await;
            }
            Err(api::SessionLoadError::Other(message)) => {
                record_session_bootstrap_failure(message, SessionLifecycle::Error, signals);
                refresh_session_list(signals).await;
            }
        }
    });
}

pub(super) fn bind_slash_completion(signals: SessionSignals) {
    Effect::new(move |_| {
        let draft = signals.draft.get();
        let candidates = local_slash_candidates(&draft);
        if candidates.is_empty() {
            dismiss_slash_palette(signals);
        } else {
            signals.slash.candidates.set(candidates);
            signals.slash.selected_index.set(0);
        }
    });
}

pub(super) fn slash_palette_callbacks(signals: SessionSignals) -> ComposerSlashCallbacks {
    ComposerSlashCallbacks {
        select_next: Callback::new(move |()| {
            let next_index = cycle_slash_selection(
                signals.slash.candidates.get_untracked().len(),
                signals.slash.selected_index.get_untracked(),
                true,
            );
            signals.slash.selected_index.set(next_index);
        }),
        select_previous: Callback::new(move |()| {
            let next_index = cycle_slash_selection(
                signals.slash.candidates.get_untracked().len(),
                signals.slash.selected_index.get_untracked(),
                false,
            );
            signals.slash.selected_index.set(next_index);
        }),
        apply_selected: Callback::new(move |()| apply_selected_slash_candidate(signals)),
        apply_index: Callback::new(move |index: usize| apply_slash_candidate_at(signals, index)),
        dismiss: Callback::new(move |()| dismiss_slash_palette(signals)),
    }
}

pub(super) fn session_permission_callbacks(
    session_id: String,
    signals: SessionSignals,
) -> (Callback<String>, Callback<String>, Callback<()>) {
    (
        permission_resolution_callback(session_id.clone(), PermissionDecision::Approve, signals),
        permission_resolution_callback(session_id.clone(), PermissionDecision::Deny, signals),
        cancel_turn_callback(session_id, signals),
    )
}

pub(super) fn session_submit_callback(
    session_id: String,
    signals: SessionSignals,
) -> Callback<String> {
    Callback::new(move |prompt: String| {
        let session_id = session_id.clone();
        if prompt.starts_with('/') {
            handle_slash_submit(&prompt, signals);
            return;
        }

        signals.turn_state.set(TurnState::Submitting);
        signals.action_error.set(None);
        dismiss_slash_palette(signals);
        leptos::task::spawn_local(async move {
            match api::send_message(&session_id, &prompt).await {
                Ok(()) => {
                    clear_prepared_session_id();
                    clear_draft(&session_id);
                    signals.draft.set(String::new());
                    signals.turn_state.set(TurnState::AwaitingReply);
                    refresh_session_list(signals).await;
                }
                Err(message) => {
                    signals.action_error.set(Some(message));
                    signals.turn_state.set(TurnState::Idle);
                }
            }
        });
    })
}

pub(super) fn rename_session_callback(signals: SessionSignals) -> Callback<(String, String)> {
    Callback::new(move |(session_id, new_title): (String, String)| {
        let new_title = new_title.trim().to_string();
        if new_title.is_empty() {
            signals.list.rename_draft.set(String::new());
            signals.list.renaming_id.set(None);
            return;
        }
        signals.list.error.set(None);
        signals.list.saving_rename_id.set(Some(session_id.clone()));
        leptos::task::spawn_local(async move {
            match api::rename_session(&session_id, &new_title).await {
                Ok(session) => {
                    signals.list.items.update(|list| {
                        rename_session_in_list(list, &session_id, session.title);
                    });
                    signals.list.rename_draft.set(String::new());
                    signals.list.renaming_id.set(None);
                }
                Err(message) => {
                    signals.list.error.set(Some(message));
                    signals.list.rename_draft.set(new_title.clone());
                    signals.list.renaming_id.set(Some(session_id.clone()));
                }
            }
            signals.list.saving_rename_id.set(None);
        });
    })
}

pub(super) fn delete_session_callback(
    current_session_id: String,
    signals: SessionSignals,
) -> Callback<String> {
    Callback::new(move |session_id: String| {
        if delete_session_is_blocked(&session_id, &current_session_id, signals) {
            return;
        }

        signals.list.deleting_id.set(Some(session_id.clone()));
        signals.list.error.set(None);
        let is_deleting_current = session_id == current_session_id;

        leptos::task::spawn_local(async move {
            match api::delete_session(&session_id).await {
                Ok(_) => {
                    clear_prepared_session_id_if_matches(&session_id);
                    clear_draft(&session_id);
                    signals
                        .list
                        .items
                        .update(|list| remove_session_from_list(list, &session_id));
                    if is_deleting_current {
                        finish_current_session_delete(signals);
                    } else {
                        finish_other_session_delete(signals).await;
                    }
                }
                Err(message) => handle_delete_session_error(message, signals),
            }
        });
    })
}

async fn refresh_session_list(signals: SessionSignals) {
    signals.list.error.set(None);

    match api::list_sessions().await {
        Ok(sessions) => {
            signals.list.items.set(sessions);
            signals.list.loaded.set(true);
        }
        Err(message) => {
            signals.list.loaded.set(true);
            signals.list.error.set(Some(message));
        }
    }
}

fn apply_selected_slash_candidate(signals: SessionSignals) {
    let index = signals.slash.selected_index.get_untracked();
    apply_slash_candidate_at(signals, index);
}

fn apply_slash_candidate_at(signals: SessionSignals, index: usize) {
    let Some(candidate) = signals.slash.candidates.get_untracked().get(index).cloned() else {
        return;
    };
    let Some(next_draft) = apply_slash_completion(&signals.draft.get_untracked(), &candidate)
    else {
        return;
    };
    signals.draft.set(next_draft);
    signals.slash.selected_index.set(index);
}

fn dismiss_slash_palette(signals: SessionSignals) {
    signals.slash.candidates.set(Vec::new());
    signals.slash.selected_index.set(0);
}

fn handle_slash_submit(prompt: &str, signals: SessionSignals) {
    match parse_browser_slash_action(prompt) {
        Ok(action) => {
            signals.action_error.set(None);
            signals.draft.set(String::new());
            dismiss_slash_palette(signals);
            run_browser_slash_action(action, signals);
        }
        Err(message) => {
            push_tool_activity_entry(
                signals,
                next_tool_activity_id(signals, "slash"),
                "Slash command",
                message,
                Vec::new(),
            );
        }
    }
}

fn run_browser_slash_action(action: BrowserSlashAction, signals: SessionSignals) {
    match action {
        BrowserSlashAction::Help => {
            let commands = local_browser_commands();
            push_tool_activity_entry(
                signals,
                next_tool_activity_id(signals, "help"),
                "Available slash commands",
                if commands.is_empty() {
                    "No browser slash commands are available.".to_string()
                } else {
                    "Use the composer for `/help` and the on-screen controls for cancel or permission actions.".to_string()
                },
                commands,
            );
        }
    }
}

async fn navigate_home_target(target: HomeRouteTarget) -> Result<(), String> {
    match target {
        HomeRouteTarget::Register => navigate_to("/app/register/"),
        HomeRouteTarget::SignIn => navigate_to("/app/sign-in/"),
        HomeRouteTarget::PrepareSession => navigate_prepared_home_session().await,
    }
}

async fn navigate_prepared_home_session() -> Result<(), String> {
    let session_id = resolve_home_session_id().await?;
    match navigate_to(&app_session_path(&session_id)) {
        Ok(()) => Ok(()),
        Err(message) => {
            clear_prepared_session_id();
            Err(message)
        }
    }
}

fn set_home_redirect_error(
    error: RwSignal<Option<String>>,
    preparing: RwSignal<bool>,
    message: String,
) {
    error.set(Some(message));
    preparing.set(false);
}

async fn resolve_home_session_id() -> Result<String, String> {
    if let Some(session_id) = prepared_session_id() {
        Ok(session_id)
    } else {
        let session_id = api::create_session().await?;
        store_prepared_session_id(&session_id);
        Ok(session_id)
    }
}

fn apply_loaded_session(session: SessionSnapshot, signals: SessionSignals) {
    let bootstrap = session_bootstrap_from_snapshot(session);
    let turn_state_for_session = turn_state_for_snapshot(&bootstrap.pending_permissions);
    let should_clear_prepared_session =
        matches!(bootstrap.session_status, SessionLifecycle::Closed)
            || bootstrap
                .entries
                .iter()
                .any(|entry| matches!(entry.role, EntryRole::User));

    signals.entries.set(bootstrap.entries);
    signals
        .pending_permissions
        .set(bootstrap.pending_permissions);
    signals.session_status.set(bootstrap.session_status);
    signals.turn_state.set(turn_state_for_session);
    if should_clear_prepared_session {
        clear_prepared_session_id();
    }
}

fn record_session_bootstrap_failure(
    message: String,
    session_lifecycle: SessionLifecycle,
    signals: SessionSignals,
) {
    clear_prepared_session_id();
    signals.connection_error.set(Some(message));
    push_tool_activity_entry(
        signals,
        next_tool_activity_id(signals, "connection"),
        "Connection",
        signals.connection_error.get_untracked().unwrap_or_default(),
        Vec::new(),
    );
    signals.session_status.set(session_lifecycle);
    signals.turn_state.set(TurnState::Idle);
}

fn permission_resolution_turn_state(decision: &PermissionDecision) -> TurnState {
    match decision {
        PermissionDecision::Approve => TurnState::AwaitingReply,
        PermissionDecision::Deny => TurnState::Idle,
    }
}

fn permission_resolution_detail(request_id: &str, decision: &PermissionDecision) -> String {
    format!(
        "{} {}.",
        request_id,
        if *decision == PermissionDecision::Approve {
            "approved"
        } else {
            "denied"
        }
    )
}

fn apply_permission_resolution_success(
    request_id: &str,
    decision: &PermissionDecision,
    signals: SessionSignals,
) {
    signals.pending_permissions.update(|current_permissions| {
        current_permissions
            .retain(|current_permission| current_permission.request_id.as_str() != request_id);
    });
    signals
        .turn_state
        .set(permission_resolution_turn_state(decision));
    push_tool_activity_entry(
        signals,
        next_tool_activity_id(signals, "permission"),
        "Permission resolved",
        permission_resolution_detail(request_id, decision),
        Vec::new(),
    );
}

async fn resolve_permission_action(
    session_id: String,
    request_id: String,
    decision: PermissionDecision,
    signals: SessionSignals,
) {
    match api::resolve_permission(&session_id, &request_id, decision.clone()).await {
        Ok(_) => {
            apply_permission_resolution_success(&request_id, &decision, signals);
            refresh_session_list(signals).await;
        }
        Err(message) => {
            signals.action_error.set(Some(message));
        }
    }
    signals.pending_action_busy.set(false);
}

fn permission_resolution_callback(
    session_id: String,
    decision: PermissionDecision,
    signals: SessionSignals,
) -> Callback<String> {
    Callback::new(move |request_id: String| {
        let session_id = session_id.clone();
        let decision = decision.clone();
        signals.pending_action_busy.set(true);
        signals.action_error.set(None);
        leptos::task::spawn_local(resolve_permission_action(
            session_id, request_id, decision, signals,
        ));
    })
}

fn cancel_turn_callback(session_id: String, signals: SessionSignals) -> Callback<()> {
    Callback::new(move |()| {
        let session_id = session_id.clone();
        let previous_turn_state = signals.turn_state.get_untracked();
        signals.pending_action_busy.set(true);
        signals.turn_state.set(TurnState::Cancelling);
        signals.action_error.set(None);
        leptos::task::spawn_local(async move {
            match api::cancel_turn(&session_id).await {
                Ok(cancelled) if cancelled.cancelled => {
                    signals.pending_permissions.set(Vec::new());
                    signals.turn_state.set(TurnState::Idle);
                    push_tool_activity_entry(
                        signals,
                        next_tool_activity_id(signals, "cancel"),
                        "Cancel turn",
                        "Cancel requested for the running turn.".to_string(),
                        Vec::new(),
                    );
                    refresh_session_list(signals).await;
                }
                Ok(_) => {
                    signals
                        .action_error
                        .set(Some("No running turn is active.".to_string()));
                    if signals.turn_state.get_untracked() == TurnState::Cancelling {
                        signals.turn_state.set(previous_turn_state);
                    }
                }
                Err(message) => {
                    signals.action_error.set(Some(message));
                    if signals.turn_state.get_untracked() == TurnState::Cancelling {
                        signals.turn_state.set(previous_turn_state);
                    }
                }
            }
            signals.pending_action_busy.set(false);
        });
    })
}

fn delete_session_is_blocked(
    session_id: &str,
    current_session_id: &str,
    signals: SessionSignals,
) -> bool {
    signals.list.deleting_id.get_untracked().is_some()
        || (session_id == current_session_id
            && session_action_busy(
                signals.turn_state.get_untracked(),
                signals.pending_action_busy.get_untracked(),
                false,
            ))
}

fn finish_current_session_delete(signals: SessionSignals) {
    let next_dest = next_session_destination(&signals.list.items.get_untracked());

    match navigate_to(&next_dest) {
        Ok(()) => stop_live_stream(signals),
        Err(message) => handle_current_session_delete_navigation_error(message, signals),
    }
}

async fn finish_other_session_delete(signals: SessionSignals) {
    refresh_session_list(signals).await;
    signals.list.deleting_id.set(None);
}

fn handle_current_session_delete_navigation_error(message: String, signals: SessionSignals) {
    stop_live_stream(signals);
    signals.pending_permissions.set(Vec::new());
    signals.turn_state.set(TurnState::Idle);
    signals.session_status.set(SessionLifecycle::Unavailable);
    signals.list.error.set(Some(message));
    signals.list.deleting_id.set(None);
}

fn handle_delete_session_error(message: String, signals: SessionSignals) {
    signals.list.error.set(Some(message));
    signals.list.deleting_id.set(None);
}

fn spawn_session_stream(session_id: String, signals: SessionSignals) {
    stop_live_stream(signals);
    let (abort_handle, abort_registration) = AbortHandle::new_pair();
    signals.stream_abort.set(Some(abort_handle));
    leptos::task::spawn_local(async move {
        let _ = Abortable::new(subscribe_sse(&session_id, signals), abort_registration).await;
        close_live_stream(signals);
        signals.stream_abort.set(None);
    });
}

async fn subscribe_sse(session_id: &str, signals: SessionSignals) {
    let (event_source, mut rx) = match api::open_session_event_stream(session_id) {
        Ok(stream) => stream,
        Err(message) => {
            signals.connection_error.set(Some(message));
            return;
        }
    };
    signals.event_source.set(Some(event_source.clone()));

    while let Some(item) = rx.next().await {
        match item {
            api::SseItem::Event(event) => {
                signals.connection_error.set(None);
                handle_sse_event(event, signals);
                if matches!(
                    signals.session_status.get_untracked(),
                    SessionLifecycle::Closed
                ) {
                    event_source.close();
                    signals.event_source.set(None);
                    return;
                }
            }
            api::SseItem::Disconnected => {
                if matches!(
                    signals.session_status.get_untracked(),
                    SessionLifecycle::Closed
                ) {
                    event_source.close();
                    signals.event_source.set(None);
                    return;
                }
                signals.connection_error.set(Some(
                    "Event stream disconnected; reconnecting...".to_string(),
                ));
            }
            api::SseItem::ParseError(message) => {
                signals.connection_error.set(Some(message));
                event_source.close();
                signals.event_source.set(None);
                return;
            }
        }
    }

    event_source.close();
    signals.event_source.set(None);
}

fn handle_sse_event(event: StreamEvent, signals: SessionSignals) {
    let StreamEvent { sequence, payload } = event;

    match payload {
        StreamEventPayload::SessionSnapshot { session } => apply_session_snapshot(session, signals),
        StreamEventPayload::ConversationMessage { message } => {
            apply_conversation_message(message, signals)
        }
        StreamEventPayload::PermissionRequested { request } => {
            apply_permission_request(request, signals)
        }
        StreamEventPayload::SessionClosed { session_id, reason } => {
            apply_session_closed(sequence, session_id, reason, signals)
        }
        StreamEventPayload::Status { message } => apply_status_update(sequence, message, signals),
    }
}

fn apply_session_snapshot(session: SessionSnapshot, signals: SessionSignals) {
    let bootstrap = session_bootstrap_from_snapshot(session);
    signals.session_status.set(bootstrap.session_status);
    if should_apply_snapshot_turn_state(signals.turn_state.get_untracked()) {
        signals
            .turn_state
            .set(turn_state_for_snapshot(&bootstrap.pending_permissions));
    }
    signals
        .pending_permissions
        .set(bootstrap.pending_permissions);
    signals.entries.set(bootstrap.entries);
}

fn apply_conversation_message(message: ConversationMessage, signals: SessionSignals) {
    let is_assistant_message = matches!(message.role, MessageRole::Assistant);
    let mut appended = false;
    signals.entries.update(|current_entries| {
        if !current_entries.iter().any(|entry| entry.id == message.id) {
            appended = true;
            current_entries.push(message_to_entry(message));
        }
    });
    if appended
        && is_assistant_message
        && should_release_turn_state(signals.turn_state.get_untracked())
    {
        signals.turn_state.set(TurnState::Idle);
    }
}

fn apply_permission_request(request: PermissionRequest, signals: SessionSignals) {
    let request_id = request.request_id;
    let summary = request.summary;
    signals.pending_permissions.update(|current_permissions| {
        if !current_permissions
            .iter()
            .any(|current_permission| current_permission.request_id.as_str() == request_id.as_str())
        {
            current_permissions.push(PendingPermission {
                request_id: request_id.clone(),
                summary: summary.clone(),
            });
        }
    });
    signals.turn_state.set(TurnState::AwaitingPermission);
    push_tool_activity_entry(
        signals,
        format!("permission-{request_id}"),
        "Permission required",
        summary,
        Vec::new(),
    );
}

fn apply_session_closed(
    sequence: u64,
    session_id: String,
    reason: String,
    signals: SessionSignals,
) {
    signals.session_status.set(SessionLifecycle::Closed);
    signals.turn_state.set(TurnState::Idle);
    signals.pending_permissions.set(Vec::new());
    signals.pending_action_busy.set(false);
    signals
        .list
        .items
        .update(|sessions| mark_session_closed(sessions, &session_id));
    push_status_entry(
        signals.entries,
        sequence,
        session_end_message(Some(&reason)),
    );
}

fn apply_status_update(sequence: u64, message: String, signals: SessionSignals) {
    if should_release_turn_state(signals.turn_state.get_untracked()) {
        signals.turn_state.set(TurnState::Idle);
    }
    push_status_entry(signals.entries, sequence, message);
}

fn push_status_entry(entries: RwSignal<Vec<TranscriptEntry>>, sequence: u64, text: String) {
    if text.trim().is_empty() {
        return;
    }

    let entry_id = format!("status-{sequence}");
    entries.update(|current_entries| {
        if current_entries.iter().any(|entry| entry.id == entry_id) {
            return;
        }

        current_entries.push(TranscriptEntry {
            id: entry_id.clone(),
            role: EntryRole::Status,
            text: text.clone(),
        });
    });
}

fn push_activity_entry(entries: RwSignal<Vec<TranscriptEntry>>, id: String, text: String) {
    if text.trim().is_empty() {
        return;
    }

    entries.update(|current_entries| {
        if current_entries.iter().any(|entry| entry.id == id) {
            return;
        }

        current_entries.push(TranscriptEntry {
            id,
            role: EntryRole::Status,
            text,
        });
    });
}

fn next_tool_activity_id(signals: SessionSignals, prefix: &str) -> String {
    let next = signals.tool_activity_serial.get_untracked() + 1;
    signals.tool_activity_serial.set(next);
    format!("{prefix}-{next}")
}

fn push_tool_activity_entry(
    signals: SessionSignals,
    id: String,
    title: impl Into<String>,
    detail: impl Into<String>,
    commands: Vec<CompletionCandidate>,
) {
    let title = title.into();
    let detail = detail.into();
    push_activity_entry(
        signals.entries,
        format!("activity-{id}"),
        tool_activity_text(&title, &detail, &commands),
    );
}

fn stop_live_stream(signals: SessionSignals) {
    if let Some(abort_handle) = signals.stream_abort.get_untracked() {
        abort_handle.abort();
        signals.stream_abort.set(None);
    }
    close_live_stream(signals);
}

fn close_live_stream(signals: SessionSignals) {
    if let Some(event_source) = signals.event_source.get_untracked() {
        event_source.close();
        signals.event_source.set(None);
    }
}
