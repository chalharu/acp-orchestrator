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

fn remove_session_from_list(
    sessions: &mut Vec<acp_contracts_sessions::SessionListItem>,
    session_id: &str,
) {
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

#[cfg(test)]
mod tests {
    use core::{
        future::Future,
        pin::pin,
        task::{Context, Poll, Waker},
    };

    use acp_contracts_permissions::PermissionRequest;
    use acp_contracts_sessions::{SessionListItem, SessionStatus};
    use chrono::{TimeZone, Utc};
    use leptos::prelude::*;

    use super::{
        delete_session_callback, delete_session_is_blocked,
        handle_current_session_delete_navigation_error, handle_delete_session_error,
        refresh_session_list, remove_session_from_list, rename_session_callback,
        rename_session_in_list,
    };
    use crate::session::page::state::session_signals;
    use crate::session_lifecycle::{SessionLifecycle, TurnState};

    fn list_item(id: &str, title: &str) -> SessionListItem {
        SessionListItem {
            id: id.to_string(),
            title: title.to_string(),
            status: SessionStatus::Active,
            last_activity_at: Utc.with_ymd_and_hms(2026, 4, 17, 1, 0, 0).unwrap(),
        }
    }

    fn permission(id: &str) -> PermissionRequest {
        PermissionRequest {
            request_id: id.to_string(),
            summary: format!("Permission for {id}"),
        }
    }

    #[test]
    fn rename_session_callback_handles_blank_and_non_blank_titles_on_host() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            let rename = rename_session_callback(signals);

            signals.list.rename_draft.set("Draft".to_string());
            signals.list.renaming_id.set(Some("session-1".to_string()));
            rename.run(("session-1".to_string(), "   ".to_string()));
            assert!(signals.list.rename_draft.get().is_empty());
            assert!(signals.list.renaming_id.get().is_none());

            rename.run(("session-2".to_string(), " New title ".to_string()));
            assert_eq!(
                signals.list.saving_rename_id.get(),
                Some("session-2".to_string())
            );
            assert_eq!(signals.list.error.get(), None);
        });
    }

    #[test]
    fn delete_session_callback_respects_blockers_on_host() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            let delete = delete_session_callback("session-1".to_string(), signals);

            signals.turn_state.set(TurnState::AwaitingReply);
            delete.run("session-1".to_string());
            assert!(signals.list.deleting_id.get().is_none());

            signals.turn_state.set(TurnState::Idle);
            delete.run("session-2".to_string());
            assert_eq!(
                signals.list.deleting_id.get(),
                Some("session-2".to_string())
            );
            assert_eq!(signals.list.error.get(), None);
        });
    }

    #[test]
    fn delete_session_blocked_checks_pending_deletes_and_busy_current_session() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            assert!(!delete_session_is_blocked(
                "session-2",
                "session-1",
                signals
            ));

            signals.list.deleting_id.set(Some("session-3".to_string()));
            assert!(delete_session_is_blocked("session-2", "session-1", signals));

            signals.list.deleting_id.set(None);
            signals.turn_state.set(TurnState::AwaitingReply);
            assert!(delete_session_is_blocked("session-1", "session-1", signals));
        });
    }

    #[test]
    fn session_list_helpers_update_errors_and_items() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals.list.items.set(vec![
                list_item("session-1", "Old"),
                list_item("session-2", "Keep"),
            ]);
            signals.list.deleting_id.set(Some("session-1".to_string()));
            signals.pending_permissions.set(vec![permission("perm")]);
            signals.turn_state.set(TurnState::AwaitingPermission);

            handle_current_session_delete_navigation_error("nav failed".to_string(), signals);
            assert_eq!(signals.session_status.get(), SessionLifecycle::Unavailable);
            assert_eq!(signals.turn_state.get(), TurnState::Idle);
            assert_eq!(signals.list.error.get(), Some("nav failed".to_string()));
            assert!(signals.list.deleting_id.get().is_none());

            handle_delete_session_error("delete failed".to_string(), signals);
            assert_eq!(signals.list.error.get(), Some("delete failed".to_string()));

            let mut items = vec![
                list_item("session-1", "Old"),
                list_item("session-2", "Keep"),
            ];
            remove_session_from_list(&mut items, "session-1");
            rename_session_in_list(&mut items, "session-2", "Renamed".to_string());
            assert_eq!(items.len(), 1);
            assert_eq!(items[0].title, "Renamed");
        });
    }

    #[test]
    fn host_refresh_session_list_completes_immediately() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            let waker = Waker::noop();
            let mut context = Context::from_waker(waker);
            let mut future = pin!(refresh_session_list(signals));

            assert!(matches!(
                Future::poll(future.as_mut(), &mut context),
                Poll::Ready(())
            ));
        });
    }
}
