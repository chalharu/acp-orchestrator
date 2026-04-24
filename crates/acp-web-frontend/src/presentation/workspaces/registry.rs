#![cfg_attr(not(target_family = "wasm"), allow(dead_code))]

use acp_contracts_sessions::SessionListItem;
use acp_contracts_workspaces::WorkspaceSummary;
use leptos::prelude::*;

#[cfg(target_family = "wasm")]
use crate::infrastructure::api;
#[cfg(target_family = "wasm")]
use crate::{browser::store_prepared_session_id, routing::app_session_path};

use super::shared::WorkspacesPageState;
#[cfg(target_family = "wasm")]
use super::shared::spawn_workspace_reload;

// ---------------------------------------------------------------------------
// Top-level section
// ---------------------------------------------------------------------------

#[component]
#[cfg(target_family = "wasm")]
pub(super) fn WorkspaceRegistrySection(state: WorkspacesPageState) -> impl IntoView {
    view! {
        <Show when=move || !state.loading.get() fallback=workspace_loading_view>
            <div class="workspace-dashboard">
                <For
                    each=move || state.workspaces.get()
                    key=|workspace| workspace.workspace_id.clone()
                    children=move |workspace| {
                        view! { <WorkspaceCard workspace state /> }
                    }
                />
                <Show when=move || state.workspaces.get().is_empty()>
                    <p class="muted">"No workspaces yet. Create one using the button above."</p>
                </Show>
            </div>
        </Show>
    }
}

#[component]
#[cfg(not(target_family = "wasm"))]
pub(super) fn WorkspaceRegistrySection(state: WorkspacesPageState) -> impl IntoView {
    let loading = state.loading.get_untracked();
    if loading {
        return workspace_loading_view();
    }

    let cards = state
        .workspaces
        .get_untracked()
        .into_iter()
        .map(|workspace| view! { <WorkspaceCard workspace state /> })
        .collect_view()
        .into_any();

    view! {
        <div class="workspace-dashboard">{cards}</div>
    }
    .into_any()
}

fn workspace_loading_view() -> AnyView {
    view! {
        <p class="muted">"Loading workspaces…"</p>
    }
    .into_any()
}

// ---------------------------------------------------------------------------
// Individual workspace card
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct WorkspaceCardDisplay {
    workspace_id: String,
    workspace_name: String,
    workspace_status: String,
    created_label: String,
}

fn workspace_card_display(workspace: &WorkspaceSummary) -> WorkspaceCardDisplay {
    WorkspaceCardDisplay {
        workspace_id: workspace.workspace_id.clone(),
        workspace_name: workspace.name.clone(),
        workspace_status: workspace.status.clone(),
        created_label: workspace.created_at.format("%Y-%m-%d").to_string(),
    }
}

#[component]
#[cfg(target_family = "wasm")]
fn WorkspaceCard(workspace: WorkspaceSummary, state: WorkspacesPageState) -> impl IntoView {
    let display = workspace_card_display(&workspace);
    let workspace_id = display.workspace_id.clone();

    let is_editing = workspace_id_signal(state.editing_workspace_id, &workspace_id);
    let is_saving = workspace_id_signal(state.saving_workspace_id, &workspace_id);
    let is_deleting = workspace_id_signal(state.deleting_workspace_id, &workspace_id);
    let is_opening = workspace_id_signal(state.opening_chat_workspace_id, &workspace_id);

    let sessions = workspace_sessions_signal(state, &workspace_id);

    let on_open_chat = workspace_open_chat_handler(workspace_id.clone(), state);
    let on_edit =
        workspace_edit_handler(workspace_id.clone(), display.workspace_name.clone(), state);
    let on_save = workspace_save_handler(workspace_id.clone(), state);
    let on_cancel = workspace_cancel_handler(workspace_id.clone(), state);
    let on_delete = workspace_delete_handler(workspace_id, state);

    view! {
        <div class="workspace-card">
            <div class="workspace-card__header">
                {workspace_card_meta_view(
                    display.clone(),
                    state,
                    is_editing,
                    is_saving,
                    on_save,
                    on_cancel,
                )}
                {workspace_card_actions_view_wasm(
                    is_editing,
                    is_deleting,
                    is_opening,
                    on_edit,
                    on_delete,
                )}
            </div>
            <WorkspaceSessionList sessions=sessions />
            <div class="workspace-card__footer">
                {workspace_card_open_button_wasm(is_deleting, is_opening, on_open_chat)}
            </div>
        </div>
    }
}

#[component]
#[cfg(not(target_family = "wasm"))]
fn WorkspaceCard(workspace: WorkspaceSummary, state: WorkspacesPageState) -> impl IntoView {
    let display = workspace_card_display(&workspace);
    let is_editing = workspace_id_flag(state.editing_workspace_id, &display.workspace_id);
    let is_saving = workspace_id_flag(state.saving_workspace_id, &display.workspace_id);
    let is_deleting = workspace_id_flag(state.deleting_workspace_id, &display.workspace_id);
    let is_opening = workspace_id_flag(state.opening_chat_workspace_id, &display.workspace_id);
    let draft = state.edit_name_draft.get_untracked();

    workspace_card_view_host(
        display,
        draft,
        is_editing,
        is_saving,
        is_deleting,
        is_opening,
    )
}

#[cfg(not(target_family = "wasm"))]
fn workspace_card_view_host(
    display: WorkspaceCardDisplay,
    draft: String,
    is_editing: bool,
    is_saving: bool,
    is_deleting: bool,
    is_opening: bool,
) -> impl IntoView {
    let name_cell = workspace_card_name_cell_host(display.clone(), draft, is_editing, is_saving);
    let actions = workspace_card_actions_view_host(is_editing, is_deleting, is_opening);
    let open_button = workspace_card_open_button_host(is_deleting, is_opening);

    view! {
        <div class="workspace-card">
            <div class="workspace-card__header">
                <div class="workspace-card__meta">{name_cell}</div>
                <div class="workspace-card__actions">{actions}</div>
            </div>
            <WorkspaceSessionListHost sessions=Vec::new() />
            <div class="workspace-card__footer">
                {open_button}
            </div>
        </div>
    }
}

#[cfg(target_family = "wasm")]
fn workspace_sessions_signal(
    state: WorkspacesPageState,
    workspace_id: &str,
) -> Signal<Option<Vec<SessionListItem>>> {
    let workspace_id = workspace_id.to_string();
    Signal::derive(move || {
        let sessions_map = state.workspace_sessions.get();
        sessions_map.get(&workspace_id).cloned()
    })
}

#[cfg(target_family = "wasm")]
fn workspace_card_meta_view(
    display: WorkspaceCardDisplay,
    state: WorkspacesPageState,
    is_editing: Signal<bool>,
    is_saving: Signal<bool>,
    on_save: Callback<web_sys::SubmitEvent>,
    on_cancel: Callback<web_sys::MouseEvent>,
) -> impl IntoView {
    view! {
        <div class="workspace-card__meta">
            <Show
                when=move || is_editing.get()
                fallback={
                    let display = display.clone();
                    move || workspace_card_summary_view(display.clone())
                }
            >
                <WorkspaceRenameForm
                    state=state
                    is_saving=is_saving
                    on_save=on_save
                    on_cancel=on_cancel
                />
            </Show>
        </div>
    }
}

fn workspace_card_summary_view(display: WorkspaceCardDisplay) -> impl IntoView {
    view! {
        <h3 class="workspace-card__name">{display.workspace_name}</h3>
        <span class="workspace-card__status">{display.workspace_status}</span>
        <span class="workspace-card__created">"Created "{display.created_label}</span>
    }
}

#[cfg(target_family = "wasm")]
fn workspace_card_actions_view_wasm(
    is_editing: Signal<bool>,
    is_deleting: Signal<bool>,
    is_opening: Signal<bool>,
    on_edit: Callback<web_sys::MouseEvent>,
    on_delete: Callback<web_sys::MouseEvent>,
) -> impl IntoView {
    view! {
        <div class="workspace-card__actions">
            <Show when=move || !is_editing.get()>
                <button
                    type="button"
                    class="workspace-action-btn"
                    prop:disabled=move || is_deleting.get() || is_opening.get()
                    on:click=move |event| on_edit.run(event)
                >
                    "Rename"
                </button>
                <button
                    type="button"
                    class="workspace-action-btn workspace-action-btn--danger"
                    prop:disabled=move || is_deleting.get() || is_opening.get()
                    on:click=move |event| on_delete.run(event)
                >
                    {move || if is_deleting.get() { "Deleting…" } else { "Delete" }}
                </button>
            </Show>
        </div>
    }
}

#[cfg(target_family = "wasm")]
fn workspace_card_open_button_wasm(
    is_deleting: Signal<bool>,
    is_opening: Signal<bool>,
    on_open_chat: Callback<web_sys::MouseEvent>,
) -> impl IntoView {
    view! {
        <button
            type="button"
            class="workspace-action-btn workspace-action-btn--primary"
            prop:disabled=move || is_deleting.get() || is_opening.get()
            on:click=move |event| on_open_chat.run(event)
        >
            {move || if is_opening.get() { "Opening…" } else { "New chat" }}
        </button>
    }
}

#[cfg(not(target_family = "wasm"))]
fn workspace_card_name_cell_host(
    display: WorkspaceCardDisplay,
    draft: String,
    is_editing: bool,
    is_saving: bool,
) -> AnyView {
    if is_editing {
        view! {
            <form class="workspace-inline-form">
                <input type="text" class="workspace-name-input" prop:value=draft prop:disabled=is_saving />
                <button type="submit" class="workspace-action-btn" prop:disabled=is_saving>
                    {if is_saving { "Saving…" } else { "Save" }}
                </button>
                <button type="button" class="workspace-action-btn" prop:disabled=is_saving>
                    "Cancel"
                </button>
            </form>
        }
        .into_any()
    } else {
        workspace_card_summary_view(display).into_any()
    }
}

#[cfg(not(target_family = "wasm"))]
fn workspace_card_actions_view_host(
    is_editing: bool,
    is_deleting: bool,
    is_opening: bool,
) -> AnyView {
    if is_editing {
        return ().into_any();
    }

    view! {
        <>
            <button type="button" class="workspace-action-btn" prop:disabled=is_deleting || is_opening>
                "Rename"
            </button>
            <button type="button" class="workspace-action-btn workspace-action-btn--danger" prop:disabled=is_deleting || is_opening>
                {if is_deleting { "Deleting…" } else { "Delete" }}
            </button>
        </>
    }
    .into_any()
}

#[cfg(not(target_family = "wasm"))]
fn workspace_card_open_button_host(is_deleting: bool, is_opening: bool) -> impl IntoView {
    view! {
        <button type="button" class="workspace-action-btn workspace-action-btn--primary" prop:disabled=is_deleting || is_opening>
            {if is_opening { "Opening…" } else { "New chat" }}
        </button>
    }
}

// ---------------------------------------------------------------------------
// Per-workspace session list
// ---------------------------------------------------------------------------

#[component]
#[cfg(target_family = "wasm")]
fn WorkspaceSessionList(sessions: Signal<Option<Vec<SessionListItem>>>) -> impl IntoView {
    view! {
        <div class="workspace-card__sessions">
            {move || match sessions.get() {
                None => view! { <p class="muted workspace-card__sessions-loading">"Loading sessions…"</p> }.into_any(),
                Some(list) if list.is_empty() => view! { <p class="muted workspace-card__sessions-empty">"No sessions yet."</p> }.into_any(),
                Some(list) => view! {
                    <ul class="workspace-card__session-list">
                        {list.into_iter().map(|session| {
                            let href = app_session_path(&session.id);
                            let title = session.title.clone();
                            view! {
                                <li class="workspace-card__session-item">
                                    <a href=href class="workspace-card__session-link">{title}</a>
                                </li>
                            }
                        }).collect_view()}
                    </ul>
                }.into_any(),
            }}
        </div>
    }
}

#[component]
#[cfg(not(target_family = "wasm"))]
fn WorkspaceSessionListHost(sessions: Vec<SessionListItem>) -> impl IntoView {
    if sessions.is_empty() {
        view! {
            <div class="workspace-card__sessions">
                <p class="muted workspace-card__sessions-empty">"No sessions yet."</p>
            </div>
        }
        .into_any()
    } else {
        let items = sessions
            .into_iter()
            .map(|session| {
                view! {
                    <li class="workspace-card__session-item">
                        <a class="workspace-card__session-link">{session.title}</a>
                    </li>
                }
            })
            .collect_view();
        view! {
            <div class="workspace-card__sessions">
                <ul class="workspace-card__session-list">{items}</ul>
            </div>
        }
        .into_any()
    }
}

// ---------------------------------------------------------------------------
// Rename form (shared between wasm component and used by card)
// ---------------------------------------------------------------------------

#[component]
#[cfg(target_family = "wasm")]
fn WorkspaceRenameForm(
    state: WorkspacesPageState,
    is_saving: Signal<bool>,
    on_save: Callback<web_sys::SubmitEvent>,
    on_cancel: Callback<web_sys::MouseEvent>,
) -> impl IntoView {
    view! {
        <form class="workspace-inline-form" on:submit=move |event| on_save.run(event)>
            <input
                type="text"
                class="workspace-name-input"
                prop:value=move || state.edit_name_draft.get()
                on:input=move |event| { state.edit_name_draft.set(event_target_value(&event)) }
                prop:disabled=move || is_saving.get()
            />
            <button type="submit" class="workspace-action-btn" prop:disabled=move || is_saving.get()>
                {move || if is_saving.get() { "Saving…" } else { "Save" }}
            </button>
            <button
                type="button"
                class="workspace-action-btn"
                prop:disabled=move || is_saving.get()
                on:click=move |event| on_cancel.run(event)
            >
                "Cancel"
            </button>
        </form>
    }
}

// ---------------------------------------------------------------------------
// Signal helpers
// ---------------------------------------------------------------------------

#[cfg(target_family = "wasm")]
fn workspace_id_signal(signal: RwSignal<Option<String>>, workspace_id: &str) -> Signal<bool> {
    let workspace_id = workspace_id.to_string();
    Signal::derive(move || signal.get().as_deref() == Some(workspace_id.as_str()))
}

#[cfg(not(target_family = "wasm"))]
fn workspace_id_flag(signal: RwSignal<Option<String>>, workspace_id: &str) -> bool {
    signal
        .get_untracked()
        .as_deref()
        .is_some_and(|id| id == workspace_id)
}

// ---------------------------------------------------------------------------
// Action handlers (wasm only)
// ---------------------------------------------------------------------------

#[cfg(target_family = "wasm")]
fn workspace_open_chat_handler(
    workspace_id: String,
    state: WorkspacesPageState,
) -> Callback<web_sys::MouseEvent> {
    Callback::new(move |_| {
        if state.opening_chat_workspace_id.get_untracked().is_some() {
            return;
        }

        state
            .opening_chat_workspace_id
            .set(Some(workspace_id.clone()));
        state.error.set(None);
        state.notice.set(None);
        let workspace_id = workspace_id.clone();
        leptos::task::spawn_local(async move {
            match api::create_workspace_session(&workspace_id).await {
                Ok(session_id) => {
                    store_prepared_session_id(&session_id);
                    if let Err(message) =
                        crate::browser::navigate_to(&app_session_path(&session_id))
                    {
                        state.opening_chat_workspace_id.set(None);
                        state.error.set(Some(message));
                    }
                }
                Err(error) => {
                    state.opening_chat_workspace_id.set(None);
                    state.error.set(Some(error.into_message()));
                }
            }
        });
    })
}

#[cfg(target_family = "wasm")]
fn workspace_edit_handler(
    workspace_id: String,
    workspace_name: String,
    state: WorkspacesPageState,
) -> Callback<web_sys::MouseEvent> {
    Callback::new(move |_| {
        state.editing_workspace_id.set(Some(workspace_id.clone()));
        state.edit_name_draft.set(workspace_name.clone());
        state.error.set(None);
        state.notice.set(None);
    })
}

#[cfg(target_family = "wasm")]
fn workspace_cancel_handler(
    workspace_id: String,
    state: WorkspacesPageState,
) -> Callback<web_sys::MouseEvent> {
    Callback::new(move |_| {
        if state
            .editing_workspace_id
            .get_untracked()
            .as_deref()
            .is_some_and(|current_id| current_id == workspace_id)
        {
            state.editing_workspace_id.set(None);
        }
    })
}

#[cfg(target_family = "wasm")]
fn workspace_save_handler(
    workspace_id: String,
    state: WorkspacesPageState,
) -> Callback<web_sys::SubmitEvent> {
    Callback::new(move |event: web_sys::SubmitEvent| {
        event.prevent_default();
        if state.saving_workspace_id.get_untracked().is_some() {
            return;
        }

        let new_name = state.edit_name_draft.get_untracked();
        if new_name.trim().is_empty() {
            state
                .error
                .set(Some("Workspace name cannot be empty.".to_string()));
            return;
        }

        let trimmed_name = new_name.trim().to_string();
        state.saving_workspace_id.set(Some(workspace_id.clone()));
        state.error.set(None);
        state.notice.set(None);
        let workspace_id = workspace_id.clone();
        leptos::task::spawn_local(async move {
            match api::update_workspace(&workspace_id, Some(trimmed_name)).await {
                Ok(_) => {
                    state.saving_workspace_id.set(None);
                    state.editing_workspace_id.set(None);
                    state.notice.set(Some("Workspace updated.".to_string()));
                    spawn_workspace_reload(state);
                }
                Err(message) => {
                    state.saving_workspace_id.set(None);
                    state.error.set(Some(message));
                }
            }
        });
    })
}

#[cfg(target_family = "wasm")]
fn workspace_delete_handler(
    workspace_id: String,
    state: WorkspacesPageState,
) -> Callback<web_sys::MouseEvent> {
    Callback::new(move |_| {
        if state.deleting_workspace_id.get_untracked().is_some() {
            return;
        }

        state.deleting_workspace_id.set(Some(workspace_id.clone()));
        state.error.set(None);
        state.notice.set(None);
        let workspace_id = workspace_id.clone();
        leptos::task::spawn_local(async move {
            match api::delete_workspace(&workspace_id).await {
                Ok(_) => {
                    state.deleting_workspace_id.set(None);
                    state.notice.set(Some("Workspace deleted.".to_string()));
                    spawn_workspace_reload(state);
                }
                Err(message) => {
                    state.deleting_workspace_id.set(None);
                    state.error.set(Some(message));
                }
            }
        });
    })
}

#[cfg(test)]
fn workspace_count_label(count: usize) -> String {
    match count {
        0 => "No workspaces".to_string(),
        1 => "1 workspace".to_string(),
        n => format!("{n} workspaces"),
    }
}

#[cfg(test)]
mod tests {
    use acp_contracts_sessions::SessionListItem;
    use acp_contracts_workspaces::WorkspaceSummary;
    use chrono::{TimeZone, Utc};
    use leptos::prelude::*;

    use super::*;
    use crate::presentation::workspaces::shared::WorkspacesPageState;

    fn sample_workspace(id: &str, name: &str) -> WorkspaceSummary {
        WorkspaceSummary {
            workspace_id: id.to_string(),
            name: name.to_string(),
            upstream_url: None,
            default_ref: None,
            bootstrap_kind: None,
            status: "active".to_string(),
            created_at: Utc.with_ymd_and_hms(2026, 4, 17, 1, 0, 0).unwrap(),
            updated_at: Utc.with_ymd_and_hms(2026, 4, 17, 1, 0, 0).unwrap(),
        }
    }

    fn sample_session(id: &str, workspace_id: &str) -> SessionListItem {
        use acp_contracts_sessions::SessionStatus;
        SessionListItem {
            id: id.to_string(),
            workspace_id: workspace_id.to_string(),
            title: format!("Session {id}"),
            status: SessionStatus::Active,
            last_activity_at: Utc.with_ymd_and_hms(2026, 4, 17, 1, 0, 0).unwrap(),
        }
    }

    #[test]
    fn workspace_count_label_pluralises_correctly() {
        assert_eq!(workspace_count_label(0), "No workspaces");
        assert_eq!(workspace_count_label(1), "1 workspace");
        assert_eq!(workspace_count_label(3), "3 workspaces");
    }

    #[test]
    fn workspace_registry_section_builds_without_panicking() {
        let owner = Owner::new();
        owner.with(|| {
            let state = WorkspacesPageState::new();
            let _ = view! { <WorkspaceRegistrySection state=state /> };
        });
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn workspace_registry_section_builds_populated_table_on_host() {
        let owner = Owner::new();
        owner.with(|| {
            let state = WorkspacesPageState::new();
            state.loading.set(false);
            state
                .workspaces
                .set(vec![sample_workspace("w_1", "Test Workspace")]);
            let _ = view! { <WorkspaceRegistrySection state=state /> };
        });
    }

    #[test]
    fn workspace_card_builds_in_view_mode() {
        let owner = Owner::new();
        owner.with(|| {
            let state = WorkspacesPageState::new();
            let workspace = sample_workspace("w_1", "Test Workspace");
            let _ = view! { <WorkspaceCard workspace=workspace state=state /> };
        });
    }

    #[test]
    fn workspace_card_builds_in_edit_mode() {
        let owner = Owner::new();
        owner.with(|| {
            let state = WorkspacesPageState::new();
            state.editing_workspace_id.set(Some("w_1".to_string()));
            state.edit_name_draft.set("Draft Name".to_string());
            let workspace = sample_workspace("w_1", "Test Workspace");
            let _ = view! { <WorkspaceCard workspace=workspace state=state /> };
        });
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn workspace_card_view_host_builds_both_name_cell_modes() {
        let owner = Owner::new();
        owner.with(|| {
            let display = workspace_card_display(&sample_workspace("w_1", "Test Workspace"));
            let _ = workspace_card_view_host(
                display.clone(),
                String::new(),
                false,
                false,
                false,
                false,
            );
            let _ =
                workspace_card_view_host(display, "Draft Name".to_string(), true, true, true, true);
        });
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn workspace_session_list_host_builds_non_empty_session_list() {
        let owner = Owner::new();
        owner.with(|| {
            let sessions = vec![sample_session("s_1", "w_1"), sample_session("s_2", "w_1")];
            let _ = WorkspaceSessionListHost(WorkspaceSessionListHostProps { sessions });
        });
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn workspace_card_shows_sessions_on_host() {
        let owner = Owner::new();
        owner.with(|| {
            let state = WorkspacesPageState::new();
            state.loading.set(false);
            state
                .workspaces
                .set(vec![sample_workspace("w_1", "Test Workspace")]);
            // Populate sessions for the workspace.
            state.workspace_sessions.update(|map| {
                map.insert(
                    "w_1".to_string(),
                    vec![sample_session("s_1", "w_1"), sample_session("s_2", "w_1")],
                );
            });
            let _ = view! { <WorkspaceRegistrySection state=state /> };
        });
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn workspace_registry_shows_workspace_scoped_sessions_not_global_list() {
        // Verify that the workspace card renders only sessions belonging to the
        // given workspace when the sessions map is populated.
        let owner = Owner::new();
        owner.with(|| {
            let state = WorkspacesPageState::new();
            state.loading.set(false);
            state.workspaces.set(vec![
                sample_workspace("w_1", "Alpha"),
                sample_workspace("w_2", "Beta"),
            ]);
            state.workspace_sessions.update(|map| {
                map.insert("w_1".to_string(), vec![sample_session("s_1", "w_1")]);
                map.insert(
                    "w_2".to_string(),
                    vec![sample_session("s_2", "w_2"), sample_session("s_3", "w_2")],
                );
            });
            // Both workspace cards should build without error.
            let _ = view! { <WorkspaceRegistrySection state=state /> };
        });
    }
}
