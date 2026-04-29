#![cfg_attr(not(target_family = "wasm"), allow(dead_code))]

use std::collections::HashMap;

use acp_contracts_sessions::SessionListItem;
use acp_contracts_workspaces::{WorkspaceBranch, WorkspaceSummary};
use leptos::prelude::*;

#[cfg(target_family = "wasm")]
use crate::infrastructure::api;

#[cfg(test)]
use crate::presentation::return_to::session_return_to_path;
use crate::{
    application::auth::WorkspacesRouteAccess,
    presentation::return_to::{path_with_return_to, session_return_to_path_from_location},
};

#[derive(Clone, Copy)]
pub(super) struct WorkspacesPageState {
    pub(super) error: RwSignal<Option<String>>,
    pub(super) notice: RwSignal<Option<String>>,
    pub(super) access: RwSignal<Option<WorkspacesRouteAccess>>,
    pub(super) workspaces: RwSignal<Vec<WorkspaceSummary>>,
    pub(super) loading: RwSignal<bool>,
    /// Sessions keyed by workspace_id; populated after the workspace list loads.
    pub(super) workspace_sessions: RwSignal<HashMap<String, Vec<SessionListItem>>>,
    pub(super) show_create_modal: RwSignal<bool>,
    pub(super) create_name: RwSignal<String>,
    pub(super) create_upstream_url: RwSignal<String>,
    pub(super) creating: RwSignal<bool>,
    pub(super) editing_workspace_id: RwSignal<Option<String>>,
    pub(super) edit_name_draft: RwSignal<String>,
    pub(super) saving_workspace_id: RwSignal<Option<String>>,
    pub(super) deleting_workspace_id: RwSignal<Option<String>>,
    pub(super) opening_chat_workspace_id: RwSignal<Option<String>>,
    pub(super) show_start_chat_modal: RwSignal<bool>,
    pub(super) start_chat_workspace_id: RwSignal<Option<String>>,
    pub(super) start_chat_workspace_name: RwSignal<String>,
    pub(super) start_chat_branches: RwSignal<Vec<WorkspaceBranch>>,
    pub(super) start_chat_selected_branch: RwSignal<String>,
    pub(super) start_chat_loading_branches: RwSignal<bool>,
    pub(super) checked: RwSignal<bool>,
}

impl WorkspacesPageState {
    pub(super) fn new() -> Self {
        Self {
            error: RwSignal::new(None::<String>),
            notice: RwSignal::new(None::<String>),
            access: RwSignal::new(None::<WorkspacesRouteAccess>),
            workspaces: RwSignal::new(Vec::<WorkspaceSummary>::new()),
            loading: RwSignal::new(true),
            workspace_sessions: RwSignal::new(HashMap::new()),
            show_create_modal: RwSignal::new(false),
            create_name: RwSignal::new(String::new()),
            create_upstream_url: RwSignal::new(String::new()),
            creating: RwSignal::new(false),
            editing_workspace_id: RwSignal::new(None::<String>),
            edit_name_draft: RwSignal::new(String::new()),
            saving_workspace_id: RwSignal::new(None::<String>),
            deleting_workspace_id: RwSignal::new(None::<String>),
            opening_chat_workspace_id: RwSignal::new(None::<String>),
            show_start_chat_modal: RwSignal::new(false),
            start_chat_workspace_id: RwSignal::new(None::<String>),
            start_chat_workspace_name: RwSignal::new(String::new()),
            start_chat_branches: RwSignal::new(Vec::<WorkspaceBranch>::new()),
            start_chat_selected_branch: RwSignal::new(String::new()),
            start_chat_loading_branches: RwSignal::new(false),
            checked: RwSignal::new(false),
        }
    }
}

pub(crate) fn default_branch_ref_name(branches: &[WorkspaceBranch]) -> String {
    branches
        .first()
        .map(|branch| branch.ref_name.clone())
        .unwrap_or_default()
}

pub(crate) fn workspaces_path_with_return_to(return_to_path: &str) -> String {
    path_with_return_to("/app/workspaces/", return_to_path)
}

#[cfg(test)]
pub(super) fn workspaces_back_to_chat_path(search: &str) -> Option<String> {
    session_return_to_path(search)
}

pub(super) fn workspaces_back_to_chat_path_from_location() -> Option<String> {
    session_return_to_path_from_location()
}

#[cfg(target_family = "wasm")]
pub(super) fn initialize_workspaces_page(state: WorkspacesPageState) {
    Effect::new(move |_| {
        if state.checked.get() {
            return;
        }

        state.checked.set(true);
        leptos::task::spawn_local(async move {
            match api::auth_status().await {
                Ok(status) => {
                    let access = crate::application::auth::workspaces_route_access(&status);
                    let should_load = matches!(access, WorkspacesRouteAccess::SignedIn);
                    state.access.set(Some(access));
                    if should_load {
                        spawn_workspace_reload(state);
                    } else {
                        state.loading.set(false);
                    }
                }
                Err(message) => {
                    state.loading.set(false);
                    state.error.set(Some(message));
                }
            }
        });
    });
}

#[cfg(not(target_family = "wasm"))]
pub(super) fn initialize_workspaces_page(state: WorkspacesPageState) {
    initialize_workspaces_page_host(state);
}

#[cfg(not(target_family = "wasm"))]
pub(super) fn initialize_workspaces_page_host(state: WorkspacesPageState) {
    if state.checked.get_untracked() {
        return;
    }

    state.checked.set(true);
    state.loading.set(false);
}

#[cfg(target_family = "wasm")]
pub(super) fn spawn_workspace_reload(state: WorkspacesPageState) {
    begin_workspace_reload(state);
    leptos::task::spawn_local(async move {
        match api::list_workspaces().await {
            Ok(workspaces) => finish_workspace_reload(state, workspaces),
            Err(message) => fail_workspace_reload(state, message),
        }
    });
}

#[cfg(not(target_family = "wasm"))]
pub(super) fn spawn_workspace_reload(state: WorkspacesPageState) {
    begin_workspace_reload(state);
    state.loading.set(false);
}

fn begin_workspace_reload(state: WorkspacesPageState) {
    state.loading.set(true);
    state.error.set(None);
    state.workspace_sessions.set(HashMap::new());
}

fn fail_workspace_reload(state: WorkspacesPageState, message: String) {
    state.loading.set(false);
    state.error.set(Some(message));
}

#[cfg(target_family = "wasm")]
fn finish_workspace_reload(state: WorkspacesPageState, workspaces: Vec<WorkspaceSummary>) {
    for workspace in &workspaces {
        spawn_workspace_sessions_reload(state, workspace);
    }
    state.workspaces.set(workspaces);
    state.loading.set(false);
}

#[cfg(target_family = "wasm")]
fn spawn_workspace_sessions_reload(state: WorkspacesPageState, workspace: &WorkspaceSummary) {
    let workspace_id = workspace.workspace_id.clone();
    let workspace_name = workspace.name.clone();
    leptos::task::spawn_local(async move {
        match api::list_workspace_sessions(&workspace_id).await {
            Ok(sessions) => store_workspace_sessions(state, workspace_id, sessions),
            Err(message) => {
                store_workspace_sessions_error(state, workspace_id, workspace_name, message)
            }
        }
    });
}

#[cfg(target_family = "wasm")]
fn store_workspace_sessions(
    state: WorkspacesPageState,
    workspace_id: String,
    sessions: Vec<SessionListItem>,
) {
    state.workspace_sessions.update(|map| {
        map.insert(workspace_id, sessions);
    });
}

#[cfg(target_family = "wasm")]
fn store_workspace_sessions_error(
    state: WorkspacesPageState,
    workspace_id: String,
    workspace_name: String,
    message: String,
) {
    store_workspace_sessions(state, workspace_id, Vec::new());
    state.error.set(Some(format!(
        "Failed to load sessions for workspace {workspace_name}: {message}"
    )));
}

#[cfg(test)]
mod tests {
    use leptos::prelude::*;

    use super::*;

    #[test]
    fn workspaces_page_state_starts_with_empty_defaults() {
        let owner = Owner::new();
        owner.with(|| {
            let state = WorkspacesPageState::new();
            assert!(state.error.get().is_none());
            assert!(state.notice.get().is_none());
            assert!(state.access.get().is_none());
            assert!(state.workspaces.get().is_empty());
            assert!(state.loading.get());
            assert!(state.workspace_sessions.get().is_empty());
            assert!(!state.show_create_modal.get());
            assert!(state.create_name.get().is_empty());
            assert!(state.create_upstream_url.get().is_empty());
            assert!(!state.creating.get());
            assert!(state.editing_workspace_id.get().is_none());
            assert!(state.edit_name_draft.get().is_empty());
            assert!(state.saving_workspace_id.get().is_none());
            assert!(state.deleting_workspace_id.get().is_none());
            assert!(state.opening_chat_workspace_id.get().is_none());
            assert!(!state.show_start_chat_modal.get());
            assert!(state.start_chat_workspace_id.get().is_none());
            assert!(state.start_chat_workspace_name.get().is_empty());
            assert!(state.start_chat_branches.get().is_empty());
            assert!(state.start_chat_selected_branch.get().is_empty());
            assert!(!state.start_chat_loading_branches.get());
            assert!(!state.checked.get());
        });
    }

    #[test]
    fn default_branch_ref_name_prefers_the_first_branch() {
        assert!(default_branch_ref_name(&[]).is_empty());
        assert_eq!(
            default_branch_ref_name(&[
                WorkspaceBranch {
                    name: "main".to_string(),
                    ref_name: "refs/heads/main".to_string(),
                },
                WorkspaceBranch {
                    name: "release".to_string(),
                    ref_name: "refs/heads/release".to_string(),
                },
            ]),
            "refs/heads/main"
        );
    }

    #[test]
    fn workspaces_paths_preserve_only_session_routes() {
        assert_eq!(
            workspaces_path_with_return_to("/app/sessions/s%2F1"),
            "/app/workspaces/?return_to=%2Fapp%2Fsessions%2Fs%252F1"
        );
        assert_eq!(
            workspaces_back_to_chat_path("?return_to=%2Fapp%2Fsessions%2Fs%252F1"),
            Some("/app/sessions/s%2F1".to_string())
        );
        assert_eq!(workspaces_back_to_chat_path("?return_to=%2Fapp%2F"), None);
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn host_initializer_marks_state_once() {
        let owner = Owner::new();
        owner.with(|| {
            let state = WorkspacesPageState::new();
            initialize_workspaces_page_host(state);
            assert!(state.checked.get());
            assert!(!state.loading.get());
            state.loading.set(true);
            initialize_workspaces_page_host(state);
            assert!(state.loading.get());
        });
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn spawn_workspace_reload_clears_errors_and_loading() {
        let owner = Owner::new();
        owner.with(|| {
            let state = WorkspacesPageState::new();
            state.error.set(Some("old error".to_string()));
            state.workspace_sessions.update(|sessions| {
                sessions.insert("workspace-1".to_string(), Vec::new());
            });
            spawn_workspace_reload(state);
            assert!(!state.loading.get());
            assert!(state.error.get().is_none());
            assert!(state.workspace_sessions.get().is_empty());
        });
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn begin_workspace_reload_resets_loading_error_and_sessions() {
        let owner = Owner::new();
        owner.with(|| {
            let state = WorkspacesPageState::new();
            state.loading.set(false);
            state.error.set(Some("old error".to_string()));
            state.workspace_sessions.update(|sessions| {
                sessions.insert("workspace-1".to_string(), Vec::new());
            });

            begin_workspace_reload(state);

            assert!(state.loading.get());
            assert!(state.error.get().is_none());
            assert!(state.workspace_sessions.get().is_empty());
        });
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn fail_workspace_reload_sets_error_and_clears_loading() {
        let owner = Owner::new();
        owner.with(|| {
            let state = WorkspacesPageState::new();
            state.loading.set(true);

            fail_workspace_reload(state, "reload failed".to_string());

            assert!(!state.loading.get());
            assert_eq!(state.error.get(), Some("reload failed".to_string()));
        });
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn back_to_chat_path_from_location_returns_none_without_browser() {
        assert_eq!(workspaces_back_to_chat_path_from_location(), None);
    }
}
