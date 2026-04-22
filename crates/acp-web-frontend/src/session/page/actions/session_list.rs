#![cfg_attr(not(target_family = "wasm"), allow(dead_code))]

use leptos::prelude::*;

#[cfg(target_family = "wasm")]
use crate::browser::{clear_draft, clear_prepared_session_id_if_matches, navigate_to};
#[cfg(target_family = "wasm")]
use crate::infrastructure::api;
#[cfg(target_family = "wasm")]
use crate::routing::app_session_path;
use crate::session_lifecycle::{SessionLifecycle, TurnState};
use crate::session_state::session_action_busy;

use super::super::state::SessionSignals;
#[cfg(target_family = "wasm")]
use super::shared::spawn_browser_task;
use super::stream::stop_live_stream;

#[cfg(target_family = "wasm")]
pub(crate) fn rename_session_callback(signals: SessionSignals) -> Callback<(String, String)> {
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
pub(crate) fn rename_session_callback(signals: SessionSignals) -> Callback<(String, String)> {
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
pub(crate) fn delete_session_callback(
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
pub(crate) fn delete_session_callback(
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
pub(super) async fn refresh_session_list(signals: SessionSignals) {
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

#[cfg(not(target_family = "wasm"))]
pub(super) async fn refresh_session_list(_signals: SessionSignals) {}

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

fn remove_session_from_list(sessions: &mut Vec<acp_contracts_sessions::SessionListItem>, session_id: &str) {
    sessions.retain(|session| session.id != session_id);
}

#[cfg(target_family = "wasm")]
fn next_session_destination(sessions: &[acp_contracts_sessions::SessionListItem]) -> String {
    sessions
        .first()
        .map(|session| app_session_path(&session.id))
        .unwrap_or_else(|| "/app/".to_string())
}

fn rename_session_in_list(
    sessions: &mut [acp_contracts_sessions::SessionListItem],
    session_id: &str,
    title: String,
) {
    if let Some(session) = sessions.iter_mut().find(|session| session.id == session_id) {
        session.title = title;
    }
}
