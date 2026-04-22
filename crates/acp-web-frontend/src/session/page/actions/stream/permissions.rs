#![cfg_attr(not(target_family = "wasm"), allow(dead_code))]

use acp_contracts_permissions::PermissionDecision;
use leptos::prelude::*;

#[cfg(target_family = "wasm")]
use crate::infrastructure::api;
use crate::session_lifecycle::TurnState;

use super::events::{next_tool_activity_id, push_tool_activity_entry};
use super::super::super::state::SessionSignals;
#[cfg(target_family = "wasm")]
use super::super::session_list::refresh_session_list;
#[cfg(target_family = "wasm")]
use super::super::shared::spawn_browser_task;

pub(crate) fn session_permission_callbacks(
    session_id: String,
    signals: SessionSignals,
) -> (Callback<String>, Callback<String>, Callback<()>) {
    (
        permission_resolution_callback(session_id.clone(), PermissionDecision::Approve, signals),
        permission_resolution_callback(session_id.clone(), PermissionDecision::Deny, signals),
        cancel_turn_callback(session_id, signals),
    )
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
