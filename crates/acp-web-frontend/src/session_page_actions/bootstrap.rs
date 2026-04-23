#![cfg_attr(not(target_family = "wasm"), allow(dead_code))]

use acp_contracts_sessions::SessionSnapshot;
#[cfg(target_family = "wasm")]
use acp_contracts_sessions::SessionStatus;
use acp_contracts_workspaces::WorkspaceSummary;
use leptos::prelude::*;

#[cfg(target_family = "wasm")]
use super::session_list::refresh_session_list;
#[cfg(target_family = "wasm")]
use super::shared::spawn_browser_task;
#[cfg(target_family = "wasm")]
use super::stream::spawn_session_stream;
use super::stream::{next_tool_activity_id, push_tool_activity_entry};
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
use crate::session_page_bootstrap::session_bootstrap_from_snapshot;
use crate::session_page_entries::{SessionEntry, SessionEntryRole};
use crate::session_page_signals::{
    SessionSignals, clear_current_workspace, set_current_workspace_id,
};
#[cfg(target_family = "wasm")]
use crate::session_page_signals::set_current_workspace_name;
use crate::session_state::turn_state_for_snapshot;

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
                sync_current_workspace_name(signals).await;
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

fn workspace_name_by_id(workspaces: &[WorkspaceSummary], workspace_id: &str) -> Option<String> {
    workspaces
        .iter()
        .find(|workspace| workspace.workspace_id == workspace_id)
        .map(|workspace| workspace.name.clone())
}

fn apply_loaded_session(session: SessionSnapshot, signals: SessionSignals) {
    let bootstrap = session_bootstrap_from_snapshot(session);
    let turn_state_for_session = turn_state_for_snapshot(&bootstrap.pending_permissions);
    let should_clear =
        should_clear_prepared_session_on_load(bootstrap.session_status, &bootstrap.entries);
    set_current_workspace_id(bootstrap.workspace_id, signals);
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

#[cfg(target_family = "wasm")]
pub(crate) async fn sync_current_workspace_name(signals: SessionSignals) {
    let Some(workspace_id) = signals.current_workspace_id.get_untracked() else {
        return;
    };

    let workspace_name = api::list_workspaces()
        .await
        .ok()
        .and_then(|workspaces| workspace_name_by_id(&workspaces, &workspace_id));

    if signals.current_workspace_id.get_untracked().as_deref() == Some(workspace_id.as_str()) {
        set_current_workspace_name(workspace_name, signals);
    }
}

fn apply_bootstrap_failure_signals(
    message: String,
    session_lifecycle: SessionLifecycle,
    signals: SessionSignals,
) {
    clear_current_workspace(signals);
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

#[cfg(test)]
mod tests {
    use acp_contracts_messages::{ConversationMessage, MessageRole};
    use acp_contracts_permissions::PermissionRequest;
    use acp_contracts_sessions::{SessionSnapshot, SessionStatus};
    use acp_contracts_workspaces::WorkspaceSummary;
    use chrono::{TimeZone, Utc};
    use leptos::prelude::*;

    use super::{
        apply_bootstrap_failure_signals, apply_loaded_session, set_home_redirect_error,
        should_clear_prepared_session_on_load, workspace_name_by_id,
    };
    use crate::session_lifecycle::{SessionLifecycle, TurnState};
    use crate::session_page_signals::session_signals;

    fn sample_snapshot(
        status: SessionStatus,
        messages: Vec<ConversationMessage>,
    ) -> SessionSnapshot {
        SessionSnapshot {
            id: "session-1".to_string(),
            workspace_id: "w_test".to_string(),
            title: "Session".to_string(),
            status,
            latest_sequence: 2,
            messages,
            pending_permissions: vec![PermissionRequest {
                request_id: "perm-1".to_string(),
                summary: "Read README".to_string(),
            }],
        }
    }

    fn message(id: &str, role: MessageRole, text: &str) -> ConversationMessage {
        ConversationMessage {
            id: id.to_string(),
            role,
            text: text.to_string(),
            created_at: Utc.with_ymd_and_hms(2026, 4, 17, 1, 0, 0).unwrap(),
        }
    }

    #[test]
    fn home_redirect_error_clears_preparing_state() {
        let owner = Owner::new();
        owner.with(|| {
            let error = RwSignal::new(None::<String>);
            let preparing = RwSignal::new(true);

            set_home_redirect_error(error, preparing, "boom".to_string());

            assert_eq!(error.get(), Some("boom".to_string()));
            assert!(!preparing.get());
        });
    }

    #[test]
    fn prepared_session_is_cleared_for_closed_or_user_entry_sessions() {
        assert!(should_clear_prepared_session_on_load(
            SessionLifecycle::Closed,
            &[],
        ));
        assert!(should_clear_prepared_session_on_load(
            SessionLifecycle::Active,
            &[crate::session_page_entries::SessionEntry::from_message(
                message("user-1", MessageRole::User, "hello",)
            )],
        ));
        assert!(!should_clear_prepared_session_on_load(
            SessionLifecycle::Active,
            &[crate::session_page_entries::SessionEntry::from_message(
                message("assistant-1", MessageRole::Assistant, "hi",)
            )],
        ));
    }

    #[test]
    fn apply_loaded_session_updates_host_signals() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();

            apply_loaded_session(
                sample_snapshot(
                    SessionStatus::Active,
                    vec![message("assistant-1", MessageRole::Assistant, "hi")],
                ),
                signals,
            );

            assert_eq!(signals.session_status.get(), SessionLifecycle::Active);
            assert_eq!(signals.turn_state.get(), TurnState::AwaitingPermission);
            assert_eq!(signals.entries.get().len(), 1);
            assert_eq!(signals.pending_permissions.get().len(), 1);
            assert_eq!(
                signals.current_workspace_id.get(),
                Some("w_test".to_string())
            );
            assert_eq!(signals.current_workspace_name.get(), None);
        });
    }

    #[test]
    fn apply_loaded_session_handles_closed_sessions_without_panicking() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();

            apply_loaded_session(
                sample_snapshot(
                    SessionStatus::Closed,
                    vec![message("user-1", MessageRole::User, "hello")],
                ),
                signals,
            );

            assert_eq!(signals.session_status.get(), SessionLifecycle::Closed);
            assert_eq!(signals.turn_state.get(), TurnState::AwaitingPermission);
            assert_eq!(signals.entries.get().len(), 2);
            assert_eq!(
                signals.current_workspace_id.get(),
                Some("w_test".to_string())
            );
        });
    }

    #[test]
    fn bootstrap_failure_signals_record_connection_activity() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals.turn_state.set(TurnState::AwaitingReply);
            signals
                .current_workspace_id
                .set(Some("workspace-a".to_string()));
            signals
                .current_workspace_name
                .set(Some("Workspace A".to_string()));

            apply_bootstrap_failure_signals(
                "network down".to_string(),
                SessionLifecycle::Unavailable,
                signals,
            );

            assert_eq!(
                signals.connection_error.get(),
                Some("network down".to_string())
            );
            assert_eq!(signals.session_status.get(), SessionLifecycle::Unavailable);
            assert_eq!(signals.turn_state.get(), TurnState::Idle);
            assert_eq!(signals.entries.get().len(), 1);
            assert!(signals.entries.get()[0].text.contains("network down"));
            assert_eq!(signals.current_workspace_id.get(), None);
            assert_eq!(signals.current_workspace_name.get(), None);
        });
    }

    #[test]
    fn workspace_name_lookup_matches_exact_id() {
        let workspaces = vec![
            WorkspaceSummary {
                workspace_id: "workspace-a".to_string(),
                name: "Workspace A".to_string(),
                upstream_url: None,
                default_ref: Some("main".to_string()),
                bootstrap_kind: None,
                status: "active".to_string(),
                created_at: Utc.with_ymd_and_hms(2026, 4, 17, 1, 0, 0).unwrap(),
                updated_at: Utc.with_ymd_and_hms(2026, 4, 17, 1, 0, 0).unwrap(),
            },
            WorkspaceSummary {
                workspace_id: "workspace-b".to_string(),
                name: "Workspace B".to_string(),
                upstream_url: None,
                default_ref: Some("main".to_string()),
                bootstrap_kind: Some("legacy-session-routes".to_string()),
                status: "active".to_string(),
                created_at: Utc.with_ymd_and_hms(2026, 4, 17, 1, 0, 0).unwrap(),
                updated_at: Utc.with_ymd_and_hms(2026, 4, 17, 1, 0, 0).unwrap(),
            },
        ];

        assert_eq!(
            workspace_name_by_id(&workspaces, "workspace-b"),
            Some("Workspace B".to_string())
        );
        assert_eq!(workspace_name_by_id(&workspaces, "missing"), None);
    }
}
