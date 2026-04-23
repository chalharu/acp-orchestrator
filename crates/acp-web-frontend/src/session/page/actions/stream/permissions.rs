#![cfg_attr(not(target_family = "wasm"), allow(dead_code))]

use acp_contracts_permissions::PermissionDecision;
use leptos::prelude::*;

#[cfg(target_family = "wasm")]
use crate::infrastructure::api;
use crate::session_lifecycle::TurnState;

use super::super::super::state::SessionSignals;
#[cfg(target_family = "wasm")]
use super::super::session_list::refresh_session_list;
#[cfg(target_family = "wasm")]
use super::super::shared::spawn_browser_task;
use super::events::{next_tool_activity_id, push_tool_activity_entry};

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

#[cfg(test)]
mod tests {
    use acp_contracts_permissions::{PermissionDecision, PermissionRequest};
    use leptos::prelude::*;

    use super::{
        apply_permission_resolution_success, begin_cancel_turn, permission_resolution_detail,
        permission_resolution_turn_state, session_permission_callbacks,
    };
    use crate::session::page::state::session_signals;
    use crate::session_lifecycle::TurnState;

    fn permission(id: &str) -> PermissionRequest {
        PermissionRequest {
            request_id: id.to_string(),
            summary: format!("Permission for {id}"),
        }
    }

    #[test]
    fn permission_resolution_helpers_match_decisions() {
        assert_eq!(
            permission_resolution_turn_state(&PermissionDecision::Approve),
            TurnState::AwaitingReply
        );
        assert_eq!(
            permission_resolution_turn_state(&PermissionDecision::Deny),
            TurnState::Idle
        );
        assert_eq!(
            permission_resolution_detail("perm-1", &PermissionDecision::Approve),
            "perm-1 approved."
        );
        assert_eq!(
            permission_resolution_detail("perm-1", &PermissionDecision::Deny),
            "perm-1 denied."
        );
    }

    #[test]
    fn permission_resolution_success_removes_requests_and_records_activity() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals
                .pending_permissions
                .set(vec![permission("perm-1"), permission("perm-2")]);

            apply_permission_resolution_success("perm-1", &PermissionDecision::Approve, signals);

            assert_eq!(signals.pending_permissions.get().len(), 1);
            assert_eq!(signals.pending_permissions.get()[0].request_id, "perm-2");
            assert_eq!(signals.turn_state.get(), TurnState::AwaitingReply);
            assert_eq!(signals.entries.get().len(), 1);
            assert!(signals.entries.get()[0].text.contains("perm-1 approved."));
        });
    }

    #[test]
    fn permission_callbacks_update_host_busy_and_cancel_state() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            let (approve, deny, cancel) =
                session_permission_callbacks("session-1".to_string(), signals);

            signals.action_error.set(Some("old".to_string()));
            approve.run("perm-1".to_string());
            assert!(signals.pending_action_busy.get());
            assert!(signals.action_error.get().is_none());

            signals.pending_action_busy.set(false);
            signals.action_error.set(Some("old".to_string()));
            deny.run("perm-2".to_string());
            assert!(signals.pending_action_busy.get());
            assert!(signals.action_error.get().is_none());

            signals.pending_action_busy.set(false);
            signals.turn_state.set(TurnState::AwaitingReply);
            cancel.run(());
            assert!(signals.pending_action_busy.get());
            assert_eq!(signals.turn_state.get(), TurnState::Cancelling);
        });
    }

    #[test]
    fn begin_cancel_turn_preserves_the_previous_state() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals.turn_state.set(TurnState::AwaitingPermission);
            signals.action_error.set(Some("old".to_string()));

            let previous = begin_cancel_turn(signals);

            assert_eq!(previous, TurnState::AwaitingPermission);
            assert!(signals.pending_action_busy.get());
            assert_eq!(signals.turn_state.get(), TurnState::Cancelling);
            assert!(signals.action_error.get().is_none());
        });
    }
}
