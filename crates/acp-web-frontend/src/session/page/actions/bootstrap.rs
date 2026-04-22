#![cfg_attr(not(target_family = "wasm"), allow(dead_code))]

#[cfg(target_family = "wasm")]
use acp_contracts_sessions::SessionStatus;
use acp_contracts_sessions::SessionSnapshot;
use leptos::prelude::*;

#[cfg(target_family = "wasm")]
use crate::application::auth::{self, HomeRouteTarget};
use crate::browser::clear_prepared_session_id;
#[cfg(target_family = "wasm")]
use crate::browser::{navigate_to, prepared_session_id, store_prepared_session_id};
#[cfg(target_family = "wasm")]
use crate::infrastructure::api;
#[cfg(target_family = "wasm")]
use crate::routing::app_session_path;
use crate::session_lifecycle::{SessionLifecycle, TurnState};
use crate::session_state::turn_state_for_snapshot;

use super::super::bootstrap::session_bootstrap_from_snapshot;
use super::super::entries::{SessionEntry, SessionEntryRole};
use super::super::state::SessionSignals;
#[cfg(target_family = "wasm")]
use super::session_list::refresh_session_list;
#[cfg(target_family = "wasm")]
use super::shared::spawn_browser_task;
use super::stream::{next_tool_activity_id, push_tool_activity_entry};
#[cfg(target_family = "wasm")]
use super::stream::spawn_session_stream;

#[cfg(target_family = "wasm")]
pub(crate) fn spawn_home_redirect(error: RwSignal<Option<String>>, preparing: RwSignal<bool>) {
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
pub(crate) fn spawn_home_redirect(_error: RwSignal<Option<String>>, _preparing: RwSignal<bool>) {}

#[cfg(target_family = "wasm")]
pub(crate) fn spawn_session_bootstrap(session_id: String, signals: SessionSignals) {
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
pub(crate) fn spawn_session_bootstrap(_session_id: String, _signals: SessionSignals) {}

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
    entries: &[SessionEntry],
) -> bool {
    matches!(session_status, SessionLifecycle::Closed)
        || entries
            .iter()
            .any(|entry| matches!(entry.role, SessionEntryRole::User))
}

fn apply_loaded_session(session: SessionSnapshot, signals: SessionSignals) {
    let bootstrap = session_bootstrap_from_snapshot(session);
    let turn_state_for_session = turn_state_for_snapshot(&bootstrap.pending_permissions);
    let should_clear =
        should_clear_prepared_session_on_load(bootstrap.session_status, &bootstrap.entries);
    signals.entries.set(bootstrap.entries);
    signals.pending_permissions.set(bootstrap.pending_permissions);
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
