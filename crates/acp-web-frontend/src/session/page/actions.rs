#![cfg_attr(not(target_family = "wasm"), allow(dead_code))]

use acp_contracts::{
    CompletionCandidate, ConversationMessage, MessageRole, PermissionDecision, PermissionRequest,
    SessionSnapshot, StreamEvent, StreamEventPayload,
};
use core::future::Future;
use futures_util::future::AbortHandle;
#[cfg(target_family = "wasm")]
use acp_contracts::SessionStatus;
#[cfg(target_family = "wasm")]
use futures_util::{StreamExt, future::Abortable};
use leptos::prelude::*;

#[cfg(target_family = "wasm")]
use crate::application::auth::{self, HomeRouteTarget};
use crate::browser::clear_prepared_session_id;
#[cfg(target_family = "wasm")]
use crate::browser::{
    clear_draft, clear_prepared_session_id_if_matches, navigate_to, prepared_session_id,
    store_prepared_session_id,
};
use crate::components::composer::ComposerSlashCallbacks;
#[cfg(target_family = "wasm")]
use crate::domain::routing::app_session_path;
use crate::domain::session::{
    PendingPermission, SessionLifecycle, TurnState, mark_session_closed, message_to_entry,
    session_action_busy, session_bootstrap_from_snapshot, session_end_message,
    should_apply_snapshot_turn_state, should_release_turn_state, tool_activity_text,
    turn_state_for_snapshot,
};
#[cfg(target_family = "wasm")]
use crate::domain::session::{
    next_session_destination, remove_session_from_list, rename_session_in_list,
};
use crate::domain::transcript::{EntryRole, TranscriptEntry};
#[cfg(target_family = "wasm")]
use crate::infrastructure::api;
use crate::slash::{
    BrowserSlashAction, apply_slash_completion, cycle_slash_selection, local_browser_commands,
    local_slash_candidates, parse_browser_slash_action,
};

use super::state::SessionSignals;

#[cfg(target_family = "wasm")]
fn spawn_browser_task(task: impl Future<Output = ()> + 'static) {
    leptos::task::spawn_local(task);
}

#[cfg(not(target_family = "wasm"))]
#[allow(dead_code)]
fn spawn_browser_task<Task>(_task: Task)
where
    Task: Future<Output = ()> + 'static,
{
}

#[cfg(target_family = "wasm")]
pub(super) fn spawn_home_redirect(error: RwSignal<Option<String>>, preparing: RwSignal<bool>) {
    spawn_browser_task(async move {
        let result = match api::auth_status().await {
            Ok(status) => navigate_home_target(auth::home_route_target(&status)).await,
            Err(message) => Err(message),
        };

        if let Err(message) = result {
            set_home_redirect_error(error, preparing, message);
        }
    });
}

#[cfg(not(target_family = "wasm"))]
pub(super) fn spawn_home_redirect(_error: RwSignal<Option<String>>, _preparing: RwSignal<bool>) {}

#[cfg(target_family = "wasm")]
pub(super) fn spawn_session_bootstrap(session_id: String, signals: SessionSignals) {
    spawn_browser_task(async move {
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

#[cfg(not(target_family = "wasm"))]
pub(super) fn spawn_session_bootstrap(_session_id: String, _signals: SessionSignals) {}

fn update_slash_completion(signals: SessionSignals, draft: &str) {
    let candidates = local_slash_candidates(draft);
    if candidates.is_empty() {
        dismiss_slash_palette(signals);
    } else {
        signals.slash.candidates.set(candidates);
        signals.slash.selected_index.set(0);
    }
}

#[cfg(target_family = "wasm")]
pub(super) fn bind_slash_completion(signals: SessionSignals) {
    Effect::new(move |_| {
        let draft = signals.draft.get();
        update_slash_completion(signals, &draft);
    });
}

#[cfg(not(target_family = "wasm"))]
pub(super) fn bind_slash_completion(signals: SessionSignals) {
    update_slash_completion(signals, &signals.draft.get_untracked());
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

#[cfg(target_family = "wasm")]
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
        spawn_browser_task(async move {
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

#[cfg(not(target_family = "wasm"))]
pub(super) fn session_submit_callback(
    _session_id: String,
    signals: SessionSignals,
) -> Callback<String> {
    Callback::new(move |prompt: String| {
        if prompt.starts_with('/') {
            handle_slash_submit(&prompt, signals);
            return;
        }

        signals.turn_state.set(TurnState::Submitting);
        signals.action_error.set(None);
        dismiss_slash_palette(signals);
    })
}

#[cfg(target_family = "wasm")]
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
        spawn_browser_task(async move {
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

#[cfg(not(target_family = "wasm"))]
pub(super) fn rename_session_callback(signals: SessionSignals) -> Callback<(String, String)> {
    Callback::new(move |(session_id, new_title): (String, String)| {
        let new_title = new_title.trim().to_string();
        if new_title.is_empty() {
            signals.list.rename_draft.set(String::new());
            signals.list.renaming_id.set(None);
            return;
        }
        signals.list.error.set(None);
        signals.list.saving_rename_id.set(Some(session_id));
    })
}

#[cfg(target_family = "wasm")]
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

        spawn_browser_task(async move {
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

#[cfg(not(target_family = "wasm"))]
pub(super) fn delete_session_callback(
    current_session_id: String,
    signals: SessionSignals,
) -> Callback<String> {
    Callback::new(move |session_id: String| {
        if delete_session_is_blocked(&session_id, &current_session_id, signals) {
            return;
        }

        signals.list.deleting_id.set(Some(session_id));
        signals.list.error.set(None);
    })
}

#[cfg(target_family = "wasm")]
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
                available_slash_commands_detail(&commands),
                commands,
            );
        }
    }
}

fn available_slash_commands_detail(commands: &[CompletionCandidate]) -> String {
    if commands.is_empty() {
        "No browser slash commands are available.".to_string()
    } else {
        "Use the composer for `/help` and the on-screen controls for cancel or permission actions."
            .to_string()
    }
}

#[cfg(target_family = "wasm")]
async fn navigate_home_target(target: HomeRouteTarget) -> Result<(), String> {
    match target {
        HomeRouteTarget::Register => navigate_to("/app/register/"),
        HomeRouteTarget::SignIn => navigate_to("/app/sign-in/"),
        HomeRouteTarget::PrepareSession => navigate_prepared_home_session().await,
    }
}

#[cfg(target_family = "wasm")]
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

#[cfg(target_family = "wasm")]
async fn resolve_home_session_id() -> Result<String, String> {
    if let Some(session_id) = prepared_session_id() {
        Ok(session_id)
    } else {
        let session_id = api::create_session().await?;
        store_prepared_session_id(&session_id);
        Ok(session_id)
    }
}

fn should_clear_prepared_session_on_load(
    session_status: SessionLifecycle,
    entries: &[crate::domain::transcript::TranscriptEntry],
) -> bool {
    matches!(session_status, SessionLifecycle::Closed)
        || entries
            .iter()
            .any(|entry| matches!(entry.role, EntryRole::User))
}

fn apply_loaded_session(session: SessionSnapshot, signals: SessionSignals) {
    let bootstrap = session_bootstrap_from_snapshot(session);
    let turn_state_for_session = turn_state_for_snapshot(&bootstrap.pending_permissions);
    let should_clear =
        should_clear_prepared_session_on_load(bootstrap.session_status, &bootstrap.entries);
    signals.entries.set(bootstrap.entries);
    signals
        .pending_permissions
        .set(bootstrap.pending_permissions);
    signals.session_status.set(bootstrap.session_status);
    signals.turn_state.set(turn_state_for_session);
    if should_clear {
        clear_prepared_session_id();
    }
}

fn apply_bootstrap_failure_signals(
    message: String,
    session_lifecycle: SessionLifecycle,
    signals: SessionSignals,
) {
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

#[cfg(target_family = "wasm")]
fn record_session_bootstrap_failure(
    message: String,
    session_lifecycle: SessionLifecycle,
    signals: SessionSignals,
) {
    clear_prepared_session_id();
    apply_bootstrap_failure_signals(message, session_lifecycle, signals);
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

#[cfg(target_family = "wasm")]
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

#[cfg(target_family = "wasm")]
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
        spawn_browser_task(resolve_permission_action(
            session_id, request_id, decision, signals,
        ));
    })
}

#[cfg(not(target_family = "wasm"))]
fn permission_resolution_callback(
    _session_id: String,
    _decision: PermissionDecision,
    signals: SessionSignals,
) -> Callback<String> {
    Callback::new(move |_request_id: String| {
        signals.pending_action_busy.set(true);
        signals.action_error.set(None);
    })
}

fn begin_cancel_turn(signals: SessionSignals) -> TurnState {
    let previous = signals.turn_state.get_untracked();
    signals.pending_action_busy.set(true);
    signals.turn_state.set(TurnState::Cancelling);
    signals.action_error.set(None);
    previous
}

#[cfg(target_family = "wasm")]
fn cancel_turn_callback(session_id: String, signals: SessionSignals) -> Callback<()> {
    Callback::new(move |()| {
        let session_id = session_id.clone();
        let previous_turn_state = begin_cancel_turn(signals);
        spawn_browser_task(async move {
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

#[cfg(not(target_family = "wasm"))]
fn cancel_turn_callback(_session_id: String, signals: SessionSignals) -> Callback<()> {
    Callback::new(move |()| {
        begin_cancel_turn(signals);
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

#[cfg(target_family = "wasm")]
fn finish_current_session_delete(signals: SessionSignals) {
    let next_dest = next_session_destination(&signals.list.items.get_untracked());

    match navigate_to(&next_dest) {
        Ok(()) => stop_live_stream(signals),
        Err(message) => handle_current_session_delete_navigation_error(message, signals),
    }
}

#[cfg(target_family = "wasm")]
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

#[cfg(target_family = "wasm")]
fn spawn_session_stream(session_id: String, signals: SessionSignals) {
    stop_live_stream(signals);
    let (abort_handle, abort_registration) = AbortHandle::new_pair();
    signals.stream_abort.set(Some(abort_handle));
    spawn_browser_task(async move {
        let _ = Abortable::new(subscribe_sse(&session_id, signals), abort_registration).await;
        close_live_stream(signals);
        signals.stream_abort.set(None);
    });
}

#[cfg(not(target_family = "wasm"))]
fn spawn_session_stream(_session_id: String, signals: SessionSignals) {
    stop_live_stream(signals);
    let (abort_handle, _abort_registration) = AbortHandle::new_pair();
    signals.stream_abort.set(Some(abort_handle));
}

#[cfg(target_family = "wasm")]
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

#[cfg(target_family = "wasm")]
fn close_live_stream(signals: SessionSignals) {
    if let Some(event_source) = signals.event_source.get_untracked() {
        event_source.close();
        signals.event_source.set(None);
    }
}

#[cfg(not(target_family = "wasm"))]
fn close_live_stream(signals: SessionSignals) {
    signals.event_source.set(None);
}

#[cfg(test)]
mod tests {
    use acp_contracts::{
        CompletionCandidate, CompletionKind, ConversationMessage, MessageRole, PermissionDecision,
        PermissionRequest, SessionSnapshot, SessionStatus, StreamEvent, StreamEventPayload,
    };
    use leptos::prelude::*;

    use crate::domain::session::{PendingPermission, SessionLifecycle, TurnState};
    use crate::domain::transcript::{EntryRole, TranscriptEntry};
    use crate::session::page::state::session_signals;

    use super::*;

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn permission(id: &str) -> PendingPermission {
        PendingPermission {
            request_id: id.to_string(),
            summary: format!("summary for {id}"),
        }
    }

    fn help_candidate() -> CompletionCandidate {
        CompletionCandidate {
            label: "/help".to_string(),
            insert_text: "/help".to_string(),
            detail: "Show available slash commands".to_string(),
            kind: CompletionKind::Command,
        }
    }

    fn empty_snapshot(id: &str, status: SessionStatus) -> SessionSnapshot {
        SessionSnapshot {
            id: id.to_string(),
            title: "Chat".to_string(),
            status,
            latest_sequence: 0,
            messages: vec![],
            pending_permissions: vec![],
        }
    }

    fn user_message(id: &str) -> ConversationMessage {
        ConversationMessage {
            id: id.to_string(),
            role: MessageRole::User,
            text: "Hello".to_string(),
            created_at: chrono::Utc::now(),
        }
    }

    fn assistant_message(id: &str) -> ConversationMessage {
        ConversationMessage {
            id: id.to_string(),
            role: MessageRole::Assistant,
            text: "Response".to_string(),
            created_at: chrono::Utc::now(),
        }
    }

    // -----------------------------------------------------------------------
    // permission_resolution_turn_state (pure)
    // -----------------------------------------------------------------------

    #[test]
    fn permission_resolution_turn_state_maps_approve_to_awaiting_reply() {
        assert_eq!(
            permission_resolution_turn_state(&PermissionDecision::Approve),
            TurnState::AwaitingReply
        );
    }

    #[test]
    fn permission_resolution_turn_state_maps_deny_to_idle() {
        assert_eq!(
            permission_resolution_turn_state(&PermissionDecision::Deny),
            TurnState::Idle
        );
    }

    // -----------------------------------------------------------------------
    // permission_resolution_detail (pure)
    // -----------------------------------------------------------------------

    #[test]
    fn permission_resolution_detail_formats_approve_correctly() {
        assert_eq!(
            permission_resolution_detail("req_1", &PermissionDecision::Approve),
            "req_1 approved."
        );
    }

    #[test]
    fn permission_resolution_detail_formats_deny_correctly() {
        assert_eq!(
            permission_resolution_detail("req_abc", &PermissionDecision::Deny),
            "req_abc denied."
        );
    }

    // -----------------------------------------------------------------------
    // push_status_entry (signal-based)
    // -----------------------------------------------------------------------

    #[test]
    fn push_status_entry_adds_entry_with_correct_id_and_role() {
        let owner = Owner::new();
        owner.with(|| {
            let entries: RwSignal<Vec<TranscriptEntry>> = RwSignal::new(Vec::new());
            push_status_entry(entries, 42, "Connected".to_string());

            let current = entries.get();
            assert_eq!(current.len(), 1);
            assert_eq!(current[0].id, "status-42");
            assert_eq!(current[0].text, "Connected");
            assert_eq!(current[0].role, EntryRole::Status);
        });
    }

    #[test]
    fn push_status_entry_skips_blank_messages() {
        let owner = Owner::new();
        owner.with(|| {
            let entries: RwSignal<Vec<TranscriptEntry>> = RwSignal::new(Vec::new());
            push_status_entry(entries, 1, "   ".to_string());
            assert!(entries.get().is_empty());
        });
    }

    #[test]
    fn push_status_entry_skips_duplicate_ids() {
        let owner = Owner::new();
        owner.with(|| {
            let entries: RwSignal<Vec<TranscriptEntry>> = RwSignal::new(Vec::new());
            push_status_entry(entries, 5, "First".to_string());
            push_status_entry(entries, 5, "Duplicate".to_string());
            assert_eq!(entries.get().len(), 1);
            assert_eq!(entries.get()[0].text, "First");
        });
    }

    // -----------------------------------------------------------------------
    // push_activity_entry (signal-based)
    // -----------------------------------------------------------------------

    #[test]
    fn push_activity_entry_adds_entry_and_skips_blanks_and_duplicates() {
        let owner = Owner::new();
        owner.with(|| {
            let entries: RwSignal<Vec<TranscriptEntry>> = RwSignal::new(Vec::new());

            // Blank message is ignored
            push_activity_entry(entries, "act-1".to_string(), "  ".to_string());
            assert!(entries.get().is_empty());

            push_activity_entry(entries, "act-1".to_string(), "Tool started".to_string());
            assert_eq!(entries.get().len(), 1);
            assert_eq!(entries.get()[0].id, "act-1");

            // Duplicate id is ignored
            push_activity_entry(entries, "act-1".to_string(), "Second".to_string());
            assert_eq!(entries.get().len(), 1);
        });
    }

    // -----------------------------------------------------------------------
    // next_tool_activity_id (signal-based)
    // -----------------------------------------------------------------------

    #[test]
    fn next_tool_activity_id_increments_serial_and_formats_id() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            assert_eq!(signals.tool_activity_serial.get(), 0);

            let id1 = next_tool_activity_id(signals, "slash");
            assert_eq!(id1, "slash-1");
            assert_eq!(signals.tool_activity_serial.get(), 1);

            let id2 = next_tool_activity_id(signals, "permission");
            assert_eq!(id2, "permission-2");
        });
    }

    // -----------------------------------------------------------------------
    // dismiss_slash_palette (signal-based)
    // -----------------------------------------------------------------------

    #[test]
    fn dismiss_slash_palette_clears_candidates_and_resets_index() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals.slash.candidates.set(vec![help_candidate()]);
            signals.slash.selected_index.set(1);

            dismiss_slash_palette(signals);

            assert!(signals.slash.candidates.get().is_empty());
            assert_eq!(signals.slash.selected_index.get(), 0);
        });
    }

    // -----------------------------------------------------------------------
    // apply_slash_candidate_at (signal-based)
    // -----------------------------------------------------------------------

    #[test]
    fn apply_slash_candidate_at_updates_draft_and_preserves_index() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals.draft.set("/h".to_string());
            signals.slash.candidates.set(vec![help_candidate()]);
            signals.slash.selected_index.set(0);

            apply_slash_candidate_at(signals, 0);

            assert_eq!(signals.draft.get(), "/help");
            assert_eq!(signals.slash.selected_index.get(), 0);
        });
    }

    #[test]
    fn apply_slash_candidate_at_does_nothing_for_out_of_range_index() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals.draft.set("/h".to_string());
            signals.slash.candidates.set(vec![help_candidate()]);

            apply_slash_candidate_at(signals, 99);

            assert_eq!(signals.draft.get(), "/h");
        });
    }

    // -----------------------------------------------------------------------
    // apply_permission_resolution_success (signal-based)
    // -----------------------------------------------------------------------

    #[test]
    fn apply_permission_resolution_success_removes_permission_and_updates_turn_state() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals
                .pending_permissions
                .set(vec![permission("req_1"), permission("req_2")]);
            signals.turn_state.set(TurnState::AwaitingPermission);

            apply_permission_resolution_success("req_1", &PermissionDecision::Approve, signals);

            let remaining = signals.pending_permissions.get();
            assert_eq!(remaining.len(), 1);
            assert_eq!(remaining[0].request_id, "req_2");
            assert_eq!(signals.turn_state.get(), TurnState::AwaitingReply);

            // A status entry is pushed into the transcript
            let entries = signals.entries.get();
            assert_eq!(entries.len(), 1);
            assert!(entries[0].text.contains("req_1"));
        });
    }

    #[test]
    fn apply_permission_resolution_success_deny_sets_idle_turn_state() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals.pending_permissions.set(vec![permission("req_x")]);
            signals.turn_state.set(TurnState::AwaitingPermission);

            apply_permission_resolution_success("req_x", &PermissionDecision::Deny, signals);

            assert!(signals.pending_permissions.get().is_empty());
            assert_eq!(signals.turn_state.get(), TurnState::Idle);
        });
    }

    // -----------------------------------------------------------------------
    // delete_session_is_blocked (signal-based)
    // -----------------------------------------------------------------------

    #[test]
    fn delete_session_not_blocked_when_idle_and_no_active_delete() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            assert!(!delete_session_is_blocked("s1", "s2", signals));
        });
    }

    #[test]
    fn delete_session_blocked_when_another_delete_is_in_progress() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals.list.deleting_id.set(Some("s1".to_string()));
            assert!(delete_session_is_blocked("s2", "s3", signals));
        });
    }

    #[test]
    fn delete_session_blocked_for_current_session_when_turn_is_active() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals.turn_state.set(TurnState::AwaitingReply);
            // Deleting the current session is blocked when the turn is active
            assert!(delete_session_is_blocked("current", "current", signals));
            // A non-current session is not blocked by this check
            assert!(!delete_session_is_blocked("other", "current", signals));
        });
    }

    // -----------------------------------------------------------------------
    // handle_current_session_delete_navigation_error (signal-based)
    // -----------------------------------------------------------------------

    #[test]
    fn current_session_delete_navigation_error_resets_state_to_unavailable() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals.turn_state.set(TurnState::AwaitingReply);
            signals.pending_permissions.set(vec![permission("req_1")]);

            handle_current_session_delete_navigation_error(
                "Navigation failed".to_string(),
                signals,
            );

            assert_eq!(signals.session_status.get(), SessionLifecycle::Unavailable);
            assert_eq!(signals.turn_state.get(), TurnState::Idle);
            assert!(signals.pending_permissions.get().is_empty());
            assert_eq!(
                signals.list.error.get(),
                Some("Navigation failed".to_string())
            );
            assert!(signals.list.deleting_id.get().is_none());
        });
    }

    // -----------------------------------------------------------------------
    // handle_delete_session_error (signal-based)
    // -----------------------------------------------------------------------

    #[test]
    fn handle_delete_session_error_sets_list_error_and_clears_deleting_id() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals.list.deleting_id.set(Some("s1".to_string()));

            handle_delete_session_error("Delete failed".to_string(), signals);

            assert_eq!(signals.list.error.get(), Some("Delete failed".to_string()));
            assert!(signals.list.deleting_id.get().is_none());
        });
    }

    // -----------------------------------------------------------------------
    // apply_status_update + push_status_entry (signal-based)
    // -----------------------------------------------------------------------

    #[test]
    fn apply_status_update_releases_turn_state_and_pushes_entry() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals.turn_state.set(TurnState::AwaitingReply);

            apply_status_update(10, "Worker finished".to_string(), signals);

            assert_eq!(signals.turn_state.get(), TurnState::Idle);
            let entries = signals.entries.get();
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].text, "Worker finished");
        });
    }

    #[test]
    fn apply_status_update_releases_awaiting_and_cancelling_turn_state() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals.turn_state.set(TurnState::Cancelling);

            apply_status_update(1, "A status".to_string(), signals);

            // Cancelling and AwaitingReply are both released to Idle on a status update
            assert_eq!(signals.turn_state.get(), TurnState::Idle);
        });
    }

    // -----------------------------------------------------------------------
    // apply_conversation_message (signal-based)
    // -----------------------------------------------------------------------

    #[test]
    fn apply_conversation_message_appends_new_messages_and_releases_turn_state() {
        use acp_contracts::{ConversationMessage, MessageRole};

        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals.turn_state.set(TurnState::AwaitingReply);

            let msg = ConversationMessage {
                id: "msg-1".to_string(),
                role: MessageRole::Assistant,
                text: "Hello!".to_string(),
                created_at: chrono::Utc::now(),
            };
            apply_conversation_message(msg, signals);

            let entries = signals.entries.get();
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].text, "Hello!");
            assert_eq!(signals.turn_state.get(), TurnState::Idle);
        });
    }

    #[test]
    fn apply_conversation_message_skips_duplicate_ids() {
        use acp_contracts::{ConversationMessage, MessageRole};

        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            let msg = ConversationMessage {
                id: "msg-1".to_string(),
                role: MessageRole::User,
                text: "Prompt".to_string(),
                created_at: chrono::Utc::now(),
            };
            apply_conversation_message(msg.clone(), signals);
            apply_conversation_message(msg, signals);

            assert_eq!(signals.entries.get().len(), 1);
        });
    }

    // -----------------------------------------------------------------------
    // apply_permission_request (signal-based)
    // -----------------------------------------------------------------------

    #[test]
    fn apply_permission_request_adds_permission_and_sets_awaiting_state() {
        use acp_contracts::PermissionRequest;

        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            let request = PermissionRequest {
                request_id: "perm-1".to_string(),
                summary: "Read file".to_string(),
            };
            apply_permission_request(request, signals);

            let perms = signals.pending_permissions.get();
            assert_eq!(perms.len(), 1);
            assert_eq!(perms[0].request_id, "perm-1");
            assert_eq!(signals.turn_state.get(), TurnState::AwaitingPermission);

            // An activity entry is added
            let entries = signals.entries.get();
            assert!(!entries.is_empty());
        });
    }

    #[test]
    fn apply_permission_request_skips_duplicate_request_ids() {
        use acp_contracts::PermissionRequest;

        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            let request = PermissionRequest {
                request_id: "perm-1".to_string(),
                summary: "Read file".to_string(),
            };
            apply_permission_request(request.clone(), signals);
            apply_permission_request(request, signals);

            assert_eq!(signals.pending_permissions.get().len(), 1);
        });
    }

    // -----------------------------------------------------------------------
    // apply_session_closed (signal-based)
    // -----------------------------------------------------------------------

    #[test]
    fn apply_session_closed_marks_session_closed_and_clears_permissions() {
        use acp_contracts::SessionListItem;

        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals.pending_permissions.set(vec![permission("req_1")]);
            signals.turn_state.set(TurnState::AwaitingReply);
            signals.list.items.set(vec![SessionListItem {
                id: "s1".to_string(),
                title: "Chat".to_string(),
                status: acp_contracts::SessionStatus::Active,
                last_activity_at: chrono::Utc::now(),
            }]);

            apply_session_closed(20, "s1".to_string(), String::new(), signals);

            assert_eq!(signals.session_status.get(), SessionLifecycle::Closed);
            assert_eq!(signals.turn_state.get(), TurnState::Idle);
            assert!(signals.pending_permissions.get().is_empty());
            assert!(!signals.pending_action_busy.get());

            let closed_item = &signals.list.items.get()[0];
            assert_eq!(closed_item.status, acp_contracts::SessionStatus::Closed);

            let entries = signals.entries.get();
            assert!(!entries.is_empty());
        });
    }

    // -----------------------------------------------------------------------
    // apply_session_snapshot (signal-based)
    // -----------------------------------------------------------------------

    #[test]
    fn apply_session_snapshot_sets_session_status_and_entries() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            let snapshot = empty_snapshot("s1", SessionStatus::Active);

            apply_session_snapshot(snapshot, signals);

            assert_eq!(signals.session_status.get(), SessionLifecycle::Active);
            assert!(signals.entries.get().is_empty());
            assert!(signals.pending_permissions.get().is_empty());
        });
    }

    #[test]
    fn apply_session_snapshot_skips_turn_state_update_when_submitting() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals.turn_state.set(TurnState::Submitting);
            let snapshot = empty_snapshot("s1", SessionStatus::Active);

            apply_session_snapshot(snapshot, signals);

            assert_eq!(signals.turn_state.get(), TurnState::Submitting);
        });
    }

    // -----------------------------------------------------------------------
    // apply_loaded_session (signal-based)
    // Note: apply_loaded_session calls clear_prepared_session_id() when the
    // session is Closed or has User messages. Tests use non-user messages and
    // Active status to avoid triggering browser sessionStorage APIs that are
    // unavailable in native test builds.
    // -----------------------------------------------------------------------

    #[test]
    fn apply_loaded_session_populates_signals_from_snapshot() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            // Use an assistant message to avoid triggering clear_prepared_session_id
            // (which requires browser sessionStorage and panics in native tests).
            let mut snapshot = empty_snapshot("s1", SessionStatus::Active);
            snapshot.messages.push(assistant_message("msg-1"));

            apply_loaded_session(snapshot, signals);

            assert_eq!(signals.session_status.get(), SessionLifecycle::Active);
            assert_eq!(signals.entries.get().len(), 1);
        });
    }

    // -----------------------------------------------------------------------
    // record_session_bootstrap_failure: skipped in native tests
    // record_session_bootstrap_failure unconditionally calls
    // clear_prepared_session_id() which requires browser sessionStorage.
    // Coverage for this path comes from WASM integration tests.
    // -----------------------------------------------------------------------

    // -----------------------------------------------------------------------
    // slash_palette_callbacks (signal-based)
    // -----------------------------------------------------------------------

    #[test]
    fn slash_palette_callbacks_select_next_cycles_forward_through_candidates() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals
                .slash
                .candidates
                .set(vec![help_candidate(), help_candidate()]);
            signals.slash.selected_index.set(0);

            let callbacks = slash_palette_callbacks(signals);
            callbacks.select_next.run(());

            assert_eq!(signals.slash.selected_index.get(), 1);
        });
    }

    #[test]
    fn slash_palette_callbacks_select_previous_cycles_backward() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals
                .slash
                .candidates
                .set(vec![help_candidate(), help_candidate()]);
            signals.slash.selected_index.set(1);

            let callbacks = slash_palette_callbacks(signals);
            callbacks.select_previous.run(());

            assert_eq!(signals.slash.selected_index.get(), 0);
        });
    }

    #[test]
    fn slash_palette_callbacks_dismiss_clears_candidates_and_index() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals.slash.candidates.set(vec![help_candidate()]);
            signals.slash.selected_index.set(0);

            let callbacks = slash_palette_callbacks(signals);
            callbacks.dismiss.run(());

            assert!(signals.slash.candidates.get().is_empty());
            assert_eq!(signals.slash.selected_index.get(), 0);
        });
    }

    #[test]
    fn slash_palette_callbacks_apply_index_applies_matching_candidate() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals.draft.set("/h".to_string());
            signals.slash.candidates.set(vec![help_candidate()]);

            let callbacks = slash_palette_callbacks(signals);
            callbacks.apply_index.run(0);

            assert_eq!(signals.draft.get(), "/help");
        });
    }

    #[test]
    fn slash_palette_callbacks_apply_selected_applies_current_index() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals.draft.set("/h".to_string());
            signals.slash.candidates.set(vec![help_candidate()]);
            signals.slash.selected_index.set(0);

            let callbacks = slash_palette_callbacks(signals);
            callbacks.apply_selected.run(());

            assert_eq!(signals.draft.get(), "/help");
        });
    }

    // -----------------------------------------------------------------------
    // apply_selected_slash_candidate (signal-based)
    // -----------------------------------------------------------------------

    #[test]
    fn apply_selected_slash_candidate_uses_current_selected_index() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals.draft.set("/h".to_string());
            signals.slash.candidates.set(vec![help_candidate()]);
            signals.slash.selected_index.set(0);

            apply_selected_slash_candidate(signals);

            assert_eq!(signals.draft.get(), "/help");
        });
    }

    // -----------------------------------------------------------------------
    // handle_slash_submit (signal-based)
    // -----------------------------------------------------------------------

    #[test]
    fn handle_slash_submit_help_clears_draft_and_adds_activity_entry() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals.draft.set("/help".to_string());
            signals.action_error.set(Some("prior error".to_string()));

            handle_slash_submit("/help", signals);

            assert!(signals.draft.get().is_empty());
            assert!(signals.action_error.get().is_none());
            assert!(!signals.entries.get().is_empty());
        });
    }

    #[test]
    fn handle_slash_submit_unknown_command_pushes_error_entry() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();

            handle_slash_submit("/unknown-command-xyz", signals);

            let entries = signals.entries.get();
            assert!(!entries.is_empty());
        });
    }

    // -----------------------------------------------------------------------
    // push_tool_activity_entry (signal-based)
    // -----------------------------------------------------------------------

    #[test]
    fn push_tool_activity_entry_adds_formatted_activity_entry() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();

            push_tool_activity_entry(signals, "test-1".to_string(), "Title", "Detail", vec![]);

            let entries = signals.entries.get();
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].id, "activity-test-1");
            assert_eq!(entries[0].role, EntryRole::Status);
            assert!(entries[0].text.contains("Title"));
        });
    }

    // -----------------------------------------------------------------------
    // handle_sse_event (signal-based)
    // -----------------------------------------------------------------------

    #[test]
    fn handle_sse_event_session_snapshot_sets_status() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            let snapshot = empty_snapshot("s1", SessionStatus::Active);
            let event = StreamEvent::snapshot(snapshot);

            handle_sse_event(event, signals);

            assert_eq!(signals.session_status.get(), SessionLifecycle::Active);
        });
    }

    #[test]
    fn handle_sse_event_conversation_message_appends_entry() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            let event = StreamEvent {
                sequence: 1,
                payload: StreamEventPayload::ConversationMessage {
                    message: user_message("msg-1"),
                },
            };

            handle_sse_event(event, signals);

            assert_eq!(signals.entries.get().len(), 1);
        });
    }

    #[test]
    fn handle_sse_event_permission_requested_adds_permission() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            let event = StreamEvent {
                sequence: 2,
                payload: StreamEventPayload::PermissionRequested {
                    request: PermissionRequest {
                        request_id: "perm-1".to_string(),
                        summary: "Read file".to_string(),
                    },
                },
            };

            handle_sse_event(event, signals);

            assert_eq!(signals.pending_permissions.get().len(), 1);
            assert_eq!(signals.turn_state.get(), TurnState::AwaitingPermission);
        });
    }

    #[test]
    fn handle_sse_event_status_message_pushes_status_entry() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            let event = StreamEvent::status(3, "Worker finished");

            handle_sse_event(event, signals);

            let entries = signals.entries.get();
            assert_eq!(entries.len(), 1);
            assert!(entries[0].text.contains("Worker finished"));
        });
    }

    #[test]
    fn handle_sse_event_session_closed_marks_session_as_closed() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            let event = StreamEvent {
                sequence: 10,
                payload: StreamEventPayload::SessionClosed {
                    session_id: "s1".to_string(),
                    reason: "User ended session".to_string(),
                },
            };

            handle_sse_event(event, signals);

            assert_eq!(signals.session_status.get(), SessionLifecycle::Closed);
        });
    }

    // -----------------------------------------------------------------------
    // should_clear_prepared_session_on_load (pure)
    // -----------------------------------------------------------------------

    #[test]
    fn should_clear_prepared_session_on_load_true_for_closed_session() {
        use crate::domain::session::session_bootstrap_from_snapshot;
        let snapshot = empty_snapshot("s1", SessionStatus::Closed);
        let bootstrap = session_bootstrap_from_snapshot(snapshot);
        assert!(should_clear_prepared_session_on_load(
            bootstrap.session_status,
            &bootstrap.entries
        ));
    }

    #[test]
    fn should_clear_prepared_session_on_load_true_when_user_message_present() {
        use crate::domain::session::session_bootstrap_from_snapshot;
        let mut snapshot = empty_snapshot("s1", SessionStatus::Active);
        snapshot.messages.push(user_message("msg-u1"));
        let bootstrap = session_bootstrap_from_snapshot(snapshot);
        assert!(should_clear_prepared_session_on_load(
            bootstrap.session_status,
            &bootstrap.entries
        ));
    }

    #[test]
    fn should_clear_prepared_session_on_load_false_for_active_with_no_user_messages() {
        use crate::domain::session::session_bootstrap_from_snapshot;
        let mut snapshot = empty_snapshot("s1", SessionStatus::Active);
        snapshot.messages.push(assistant_message("msg-a1"));
        let bootstrap = session_bootstrap_from_snapshot(snapshot);
        assert!(!should_clear_prepared_session_on_load(
            bootstrap.session_status,
            &bootstrap.entries
        ));
    }

    // -----------------------------------------------------------------------
    // apply_bootstrap_failure_signals (signal-based)
    // -----------------------------------------------------------------------

    #[test]
    fn apply_bootstrap_failure_signals_sets_connection_error_and_resets_state() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals.turn_state.set(TurnState::AwaitingReply);

            apply_bootstrap_failure_signals(
                "Connection lost".to_string(),
                SessionLifecycle::Error,
                signals,
            );

            assert_eq!(
                signals.connection_error.get(),
                Some("Connection lost".to_string())
            );
            assert_eq!(signals.session_status.get(), SessionLifecycle::Error);
            assert_eq!(signals.turn_state.get(), TurnState::Idle);
            assert!(!signals.entries.get().is_empty());
        });
    }

    // -----------------------------------------------------------------------
    // set_home_redirect_error (signal-based)
    // -----------------------------------------------------------------------

    #[test]
    fn set_home_redirect_error_sets_error_and_clears_preparing_flag() {
        let owner = Owner::new();
        owner.with(|| {
            let error: RwSignal<Option<String>> = RwSignal::new(None);
            let preparing = RwSignal::new(true);

            set_home_redirect_error(error, preparing, "Auth failed".to_string());

            assert_eq!(error.get(), Some("Auth failed".to_string()));
            assert!(!preparing.get());
        });
    }

    #[test]
    fn host_spawn_helpers_are_safe_noops() {
        let owner = Owner::new();
        owner.with(|| {
            let home_error: RwSignal<Option<String>> = RwSignal::new(None);
            let home_preparing = RwSignal::new(true);
            spawn_home_redirect(home_error, home_preparing);
            assert!(home_error.get().is_none());
            assert!(home_preparing.get());

            let signals = session_signals();
            spawn_session_bootstrap("session-1".to_string(), signals);
            assert_eq!(signals.session_status.get(), SessionLifecycle::Loading);
            assert!(signals.list.items.get().is_empty());
            assert!(signals.stream_abort.get().is_none());
        });
    }

    // -----------------------------------------------------------------------
    // stop_live_stream / close_live_stream (signal-based, no-op paths)
    // -----------------------------------------------------------------------

    #[test]
    fn stop_and_close_live_stream_are_noops_when_no_stream_or_source_set() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            stop_live_stream(signals);
            close_live_stream(signals);
            assert!(signals.stream_abort.get().is_none());
            assert!(signals.event_source.get().is_none());
        });
    }

    // -----------------------------------------------------------------------
    // rename_session_callback – empty title early return (signal-based)
    // -----------------------------------------------------------------------

    #[test]
    fn rename_session_callback_clears_draft_and_renaming_id_for_blank_title() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals.list.rename_draft.set("old draft".to_string());
            signals.list.renaming_id.set(Some("s1".to_string()));

            let callback = rename_session_callback(signals);
            callback.run(("s1".to_string(), "  ".to_string()));

            assert!(signals.list.rename_draft.get().is_empty());
            assert!(signals.list.renaming_id.get().is_none());
        });
    }

    // -----------------------------------------------------------------------
    // delete_session_callback – blocked early return (signal-based)
    // -----------------------------------------------------------------------

    #[test]
    fn delete_session_callback_does_not_start_delete_when_blocked() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals.list.deleting_id.set(Some("other".to_string()));

            let callback = delete_session_callback("current".to_string(), signals);
            callback.run("s1".to_string());

            assert_eq!(signals.list.deleting_id.get(), Some("other".to_string()));
        });
    }

    // -----------------------------------------------------------------------
    // session_submit_callback – slash-command synchronous path (signal-based)
    // -----------------------------------------------------------------------

    #[test]
    fn session_submit_callback_handles_slash_commands_without_async() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            let callback = session_submit_callback("s1".to_string(), signals);
            callback.run("/help".to_string());

            assert!(signals.draft.get().is_empty());
            assert!(!signals.entries.get().is_empty());
        });
    }

    #[test]
    fn session_submit_callback_updates_host_state_before_async_work() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals.draft.set("/help".to_string());
            signals.slash.candidates.set(vec![help_candidate()]);
            signals.slash.selected_index.set(2);

            let callback = session_submit_callback("s1".to_string(), signals);
            callback.run("hello".to_string());

            assert_eq!(signals.turn_state.get(), TurnState::Submitting);
            assert!(signals.action_error.get().is_none());
            assert!(signals.slash.candidates.get().is_empty());
            assert_eq!(signals.slash.selected_index.get(), 0);
        });
    }

    #[test]
    fn rename_session_callback_marks_host_save_state_before_async_work() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals.list.error.set(Some("old".to_string()));

            let callback = rename_session_callback(signals);
            callback.run(("s1".to_string(), "Renamed".to_string()));

            assert!(signals.list.error.get().is_none());
            assert_eq!(signals.list.saving_rename_id.get(), Some("s1".to_string()));
        });
    }

    #[test]
    fn delete_session_callback_marks_host_delete_state_before_async_work() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals.list.error.set(Some("old".to_string()));

            let callback = delete_session_callback("current".to_string(), signals);
            callback.run("other".to_string());

            assert_eq!(signals.list.deleting_id.get(), Some("other".to_string()));
            assert!(signals.list.error.get().is_none());
        });
    }

    // -----------------------------------------------------------------------
    // session_permission_callbacks – construction (signal-based)
    // -----------------------------------------------------------------------

    #[test]
    fn session_permission_callbacks_constructs_three_callbacks() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            let (approve_cb, deny_cb, cancel_cb) =
                session_permission_callbacks("s1".to_string(), signals);
            let _ = (approve_cb, deny_cb, cancel_cb);
        });
    }

    #[test]
    fn permission_resolution_callback_marks_busy_before_async_work() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals.action_error.set(Some("old".to_string()));

            let approve = permission_resolution_callback(
                "s1".to_string(),
                PermissionDecision::Approve,
                signals,
            );
            approve.run("req-1".to_string());

            assert!(signals.pending_action_busy.get());
            assert!(signals.action_error.get().is_none());
        });
    }

    // -----------------------------------------------------------------------
    // begin_cancel_turn (synchronous helper extracted from cancel_turn_callback)
    // -----------------------------------------------------------------------

    #[test]
    fn begin_cancel_turn_sets_cancelling_state_and_busy_and_returns_previous() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals.turn_state.set(TurnState::AwaitingReply);
            signals.action_error.set(Some("prior".to_string()));

            let previous = begin_cancel_turn(signals);

            assert_eq!(previous, TurnState::AwaitingReply);
            assert_eq!(signals.turn_state.get(), TurnState::Cancelling);
            assert!(signals.pending_action_busy.get());
            assert!(signals.action_error.get().is_none());
        });
    }

    #[test]
    fn cancel_turn_callback_marks_host_state_before_async_work() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals.turn_state.set(TurnState::AwaitingReply);
            signals.action_error.set(Some("old".to_string()));

            let cancel = cancel_turn_callback("s1".to_string(), signals);
            cancel.run(());

            assert_eq!(signals.turn_state.get(), TurnState::Cancelling);
            assert!(signals.pending_action_busy.get());
            assert!(signals.action_error.get().is_none());
        });
    }

    #[test]
    fn spawn_session_stream_sets_abort_handle_before_async_work() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();

            spawn_session_stream("s1".to_string(), signals);

            assert!(signals.stream_abort.get().is_some());
            assert!(signals.event_source.get().is_none());

            stop_live_stream(signals);
            assert!(signals.stream_abort.get().is_none());
        });
    }

    // -----------------------------------------------------------------------
    // cancel_turn_callback – construction only (no executor needed)
    // -----------------------------------------------------------------------

    #[test]
    fn cancel_turn_callback_construction_succeeds() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            let _cb = cancel_turn_callback("s1".to_string(), signals);
        });
    }

    // -----------------------------------------------------------------------
    // apply_conversation_message – additional branches (signal-based)
    // -----------------------------------------------------------------------

    #[test]
    fn apply_conversation_message_user_message_does_not_release_awaiting_reply() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals.turn_state.set(TurnState::AwaitingReply);

            apply_conversation_message(user_message("msg-u1"), signals);

            assert_eq!(signals.entries.get().len(), 1);
            assert_eq!(signals.turn_state.get(), TurnState::AwaitingReply);
        });
    }

    #[test]
    fn apply_conversation_message_assistant_message_no_release_when_already_idle() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals.turn_state.set(TurnState::Idle);

            apply_conversation_message(assistant_message("msg-a1"), signals);

            assert_eq!(signals.entries.get().len(), 1);
            assert_eq!(signals.turn_state.get(), TurnState::Idle);
        });
    }

    // -----------------------------------------------------------------------
    // apply_status_update – non-releasing Idle path (signal-based)
    // -----------------------------------------------------------------------

    #[test]
    fn apply_status_update_does_not_change_idle_turn_state() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            apply_status_update(5, "Status message".to_string(), signals);
            assert_eq!(signals.turn_state.get(), TurnState::Idle);
            assert_eq!(signals.entries.get().len(), 1);
        });
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn host_spawn_browser_task_does_not_run_the_future() {
        let owner = Owner::new();
        owner.with(|| {
            let ran = RwSignal::new(false);
            #[rustfmt::skip]
            spawn_browser_task(async move { ran.set(true); });
            assert!(!ran.get());
        });
    }

    #[test]
    fn update_slash_completion_sets_and_clears_candidates() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();

            update_slash_completion(signals, "/");
            assert!(!signals.slash.candidates.get().is_empty());
            assert_eq!(signals.slash.selected_index.get(), 0);

            update_slash_completion(signals, "plain text");
            assert!(signals.slash.candidates.get().is_empty());
            assert_eq!(signals.slash.selected_index.get(), 0);
        });
    }

    #[test]
    fn apply_slash_candidate_at_applies_help_completion_directly() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals.draft.set("/".to_string());
            signals.slash.candidates.set(vec![help_candidate()]);

            apply_slash_candidate_at(signals, 0);

            assert_eq!(signals.draft.get(), "/help");
            assert_eq!(signals.slash.selected_index.get(), 0);
        });
    }

    #[test]
    fn run_browser_slash_action_help_pushes_available_commands_entry() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();

            run_browser_slash_action(BrowserSlashAction::Help, signals);

            let entries = signals.entries.get();
            assert_eq!(entries.len(), 1);
            assert!(entries[0].text.contains("Available slash commands"));
        });
    }

    #[test]
    fn apply_loaded_session_sets_turn_state_from_pending_permissions() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            let mut session = empty_snapshot("session-1", SessionStatus::Active);
            session.pending_permissions = vec![PermissionRequest {
                request_id: "perm-1".to_string(),
                summary: "Read file".to_string(),
            }];

            apply_loaded_session(session, signals);

            assert_eq!(signals.turn_state.get(), TurnState::AwaitingPermission);
            assert_eq!(signals.pending_permissions.get().len(), 1);
        });
    }

    #[test]
    fn apply_slash_candidate_at_returns_when_completion_does_not_apply() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals.draft.set("plain text".to_string());
            signals.slash.candidates.set(vec![help_candidate()]);
            signals.slash.selected_index.set(2);

            apply_slash_candidate_at(signals, 0);

            assert_eq!(signals.draft.get(), "plain text");
            assert_eq!(signals.slash.selected_index.get(), 2);
        });
    }

    #[test]
    fn available_slash_commands_detail_covers_empty_and_non_empty_lists() {
        assert_eq!(
            available_slash_commands_detail(&[]),
            "No browser slash commands are available."
        );
        assert!(available_slash_commands_detail(&[help_candidate()]).contains("/help"));
    }

    #[test]
    fn apply_loaded_session_clears_prepared_state_for_closed_sessions() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            let session = empty_snapshot("session-1", SessionStatus::Closed);

            apply_loaded_session(session, signals);

            assert_eq!(signals.session_status.get(), SessionLifecycle::Closed);
            assert_eq!(signals.turn_state.get(), TurnState::Idle);
        });
    }
}
