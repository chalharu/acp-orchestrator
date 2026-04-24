#![cfg_attr(not(target_family = "wasm"), allow(dead_code))]

use acp_contracts_workspaces::WorkspaceSummary;
use leptos::prelude::*;

#[cfg(target_family = "wasm")]
use crate::infrastructure::api;
#[cfg(target_family = "wasm")]
use crate::{
    browser::{
        clear_prepared_session_id, clear_selected_workspace_id_if_matches,
        store_prepared_session_id, store_selected_workspace_id,
    },
    routing::app_session_path,
};

use super::shared::WorkspacesPageState;
#[cfg(target_family = "wasm")]
use super::shared::spawn_workspace_reload;

#[component]
#[cfg(target_family = "wasm")]
pub(super) fn WorkspaceRegistrySection(state: WorkspacesPageState) -> impl IntoView {
    let summary_text = Signal::derive(move || {
        workspace_registry_summary(
            &state.workspaces.get(),
            state.selected_workspace_id.get().as_deref(),
        )
    });
    let summary = view! {
        <>{move || summary_text.get()}</>
    }
    .into_any();
    let content = view! {
        <Show when=move || !state.loading.get() fallback=workspace_loading_view>
            <WorkspaceRegistryTable>
                <For
                    each=move || state.workspaces.get()
                    key=|workspace| workspace.workspace_id.clone()
                    children=move |workspace| {
                        view! { <WorkspaceRow workspace state /> }
                    }
                />
            </WorkspaceRegistryTable>
        </Show>
    }
    .into_any();

    workspace_registry_panel(summary, content)
}

#[component]
#[cfg(not(target_family = "wasm"))]
pub(super) fn WorkspaceRegistrySection(state: WorkspacesPageState) -> impl IntoView {
    let loading = state.loading.get_untracked();
    let summary = workspace_registry_summary(
        &state.workspaces.get_untracked(),
        state.selected_workspace_id.get_untracked().as_deref(),
    );
    let content = if loading {
        workspace_loading_view()
    } else {
        let rows = state
            .workspaces
            .get_untracked()
            .into_iter()
            .map(|workspace| view! { <WorkspaceRow workspace state /> })
            .collect_view()
            .into_any();
        view! {
            <WorkspaceRegistryTable>{rows}</WorkspaceRegistryTable>
        }
        .into_any()
    };

    workspace_registry_panel(view! { <>{summary}</> }.into_any(), content)
}

fn workspace_loading_view() -> AnyView {
    view! {
        <p class="muted">"Loading workspaces…"</p>
    }
    .into_any()
}

fn workspace_registry_panel(summary: AnyView, content: AnyView) -> impl IntoView {
    view! {
        <div class="account-panel__section account-panel__section--registry">
            <div class="account-panel__section-heading">
                <div class="account-panel__section-copy">
                    <h2>"Workspaces"</h2>
                    <p class="muted">
                        "Manage workspaces. Switch the default for new chats, start a chat, rename, or remove them below."
                    </p>
                </div>
                <p class="account-panel__summary">{summary}</p>
            </div>
            {content}
        </div>
    }
}

#[component]
fn WorkspaceRegistryTable(children: Children) -> impl IntoView {
    view! {
        <div class="account-table-wrap">
            <table class="account-table">
                <WorkspaceRegistryHead />
                <tbody>{children()}</tbody>
            </table>
        </div>
    }
}

#[component]
fn WorkspaceRegistryHead() -> impl IntoView {
    view! {
        <caption class="sr-only">"Workspace list and management controls"</caption>
        <thead>
            <tr>
                <th scope="col">"Name"</th>
                <th scope="col">"Status"</th>
                <th scope="col">"Created"</th>
                <th scope="col">"Actions"</th>
            </tr>
        </thead>
    }
}

#[derive(Clone)]
struct WorkspaceRowDisplay {
    workspace_id: String,
    workspace_name: String,
    workspace_status: String,
    created_label: String,
}

fn workspace_row_display(workspace: &WorkspaceSummary) -> WorkspaceRowDisplay {
    WorkspaceRowDisplay {
        workspace_id: workspace.workspace_id.clone(),
        workspace_name: workspace.name.clone(),
        workspace_status: workspace.status.clone(),
        created_label: workspace.created_at.format("%Y-%m-%d").to_string(),
    }
}

#[cfg(target_family = "wasm")]
#[derive(Clone, Copy)]
struct WorkspaceRowSignals {
    is_editing: Signal<bool>,
    is_saving: Signal<bool>,
    is_deleting: Signal<bool>,
    is_opening: Signal<bool>,
    is_selected: Signal<bool>,
}

#[cfg(target_family = "wasm")]
fn workspace_row_signal(signal: RwSignal<Option<String>>, workspace_id: &str) -> Signal<bool> {
    let workspace_id = workspace_id.to_string();
    Signal::derive(move || signal.get().as_deref() == Some(workspace_id.as_str()))
}

#[cfg(target_family = "wasm")]
fn workspace_row_signals(state: WorkspacesPageState, workspace_id: &str) -> WorkspaceRowSignals {
    WorkspaceRowSignals {
        is_editing: workspace_row_signal(state.editing_workspace_id, workspace_id),
        is_saving: workspace_row_signal(state.saving_workspace_id, workspace_id),
        is_deleting: workspace_row_signal(state.deleting_workspace_id, workspace_id),
        is_opening: workspace_row_signal(state.opening_chat_workspace_id, workspace_id),
        is_selected: workspace_row_signal(state.selected_workspace_id, workspace_id),
    }
}

#[cfg(not(target_family = "wasm"))]
struct WorkspaceRowFlags {
    is_editing: bool,
    is_saving: bool,
    is_deleting: bool,
    is_opening: bool,
    is_selected: bool,
}

#[cfg(not(target_family = "wasm"))]
fn workspace_row_flag(signal: RwSignal<Option<String>>, workspace_id: &str) -> bool {
    signal
        .get_untracked()
        .as_deref()
        .is_some_and(|id| id == workspace_id)
}

#[cfg(not(target_family = "wasm"))]
fn workspace_row_flags(state: WorkspacesPageState, workspace_id: &str) -> WorkspaceRowFlags {
    WorkspaceRowFlags {
        is_editing: workspace_row_flag(state.editing_workspace_id, workspace_id),
        is_saving: workspace_row_flag(state.saving_workspace_id, workspace_id),
        is_deleting: workspace_row_flag(state.deleting_workspace_id, workspace_id),
        is_opening: workspace_row_flag(state.opening_chat_workspace_id, workspace_id),
        is_selected: workspace_row_flag(state.selected_workspace_id, workspace_id),
    }
}

#[component]
#[cfg(target_family = "wasm")]
fn WorkspaceRow(workspace: WorkspaceSummary, state: WorkspacesPageState) -> impl IntoView {
    let display = workspace_row_display(&workspace);
    let row_state = workspace_row_signals(state, &display.workspace_id);

    workspace_row_view(display, state, row_state)
}

#[cfg(target_family = "wasm")]
fn workspace_row_view(
    display: WorkspaceRowDisplay,
    state: WorkspacesPageState,
    row_state: WorkspaceRowSignals,
) -> impl IntoView {
    let WorkspaceRowDisplay {
        workspace_id,
        workspace_name,
        workspace_status,
        created_label,
    } = display;

    view! {
        <tr>
            <td>
                <WorkspaceNameCell
                    state=state
                    workspace_id=workspace_id.clone()
                    workspace_name=workspace_name.clone()
                    is_editing=row_state.is_editing
                    is_saving=row_state.is_saving
                />
            </td>
            <td>{workspace_status}</td>
            <td>{created_label}</td>
            <td>
                <WorkspaceActionCell
                    state=state
                    workspace_id=workspace_id
                    workspace_name=workspace_name
                    is_editing=row_state.is_editing
                    is_deleting=row_state.is_deleting
                    is_opening=row_state.is_opening
                    is_selected=row_state.is_selected
                />
            </td>
        </tr>
    }
}

#[component]
#[cfg(target_family = "wasm")]
fn WorkspaceNameCell(
    state: WorkspacesPageState,
    workspace_id: String,
    workspace_name: String,
    is_editing: Signal<bool>,
    is_saving: Signal<bool>,
) -> impl IntoView {
    let on_save = workspace_save_handler(workspace_id.clone(), state);
    let on_cancel = workspace_cancel_handler(workspace_id, state);

    view! {
        <Show
            when=move || is_editing.get()
            fallback={
                let workspace_name = workspace_name.clone();
                move || view! { <span>{workspace_name.clone()}</span> }
            }
        >
            <WorkspaceRenameForm
                state=state
                is_saving=is_saving
                on_save=on_save
                on_cancel=on_cancel
            />
        </Show>
    }
}

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
                on:input=move |event| {
                    state.edit_name_draft.set(event_target_value(&event))
                }
                prop:disabled=move || is_saving.get()
            />
            <button
                type="submit"
                class="workspace-action-btn"
                prop:disabled=move || is_saving.get()
            >
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

#[component]
#[cfg(target_family = "wasm")]
fn WorkspaceActionCell(
    state: WorkspacesPageState,
    workspace_id: String,
    workspace_name: String,
    is_editing: Signal<bool>,
    is_deleting: Signal<bool>,
    is_opening: Signal<bool>,
    is_selected: Signal<bool>,
) -> impl IntoView {
    let on_open_chat = workspace_open_chat_handler(workspace_id.clone(), state);
    let on_select = workspace_select_handler(workspace_id.clone(), workspace_name.clone(), state);
    let on_edit = workspace_edit_handler(workspace_id.clone(), workspace_name, state);
    let on_delete = workspace_delete_handler(workspace_id, state);

    view! {
        <Show when=move || !is_editing.get()>
            <WorkspaceActionButtons
                is_deleting=is_deleting
                is_opening=is_opening
                is_selected=is_selected
                on_open_chat=on_open_chat
                on_select=on_select
                on_edit=on_edit
                on_delete=on_delete
            />
        </Show>
    }
}

#[component]
#[cfg(target_family = "wasm")]
fn WorkspaceActionButtons(
    is_deleting: Signal<bool>,
    is_opening: Signal<bool>,
    is_selected: Signal<bool>,
    on_open_chat: Callback<web_sys::MouseEvent>,
    on_select: Callback<web_sys::MouseEvent>,
    on_edit: Callback<web_sys::MouseEvent>,
    on_delete: Callback<web_sys::MouseEvent>,
) -> impl IntoView {
    let actions_disabled = Signal::derive(move || is_deleting.get() || is_opening.get());

    view! {
        <>
            <WorkspaceOpenChatButton
                actions_disabled=actions_disabled
                is_opening=is_opening
                on_click=on_open_chat
            />
            <WorkspaceSelectButton
                actions_disabled=actions_disabled
                is_selected=is_selected
                on_click=on_select
            />
            <WorkspaceRenameButton actions_disabled=actions_disabled on_click=on_edit />
            <WorkspaceDeleteButton
                actions_disabled=actions_disabled
                is_deleting=is_deleting
                on_click=on_delete
            />
        </>
    }
}

#[cfg(target_family = "wasm")]
#[component]
fn WorkspaceOpenChatButton(
    actions_disabled: Signal<bool>,
    is_opening: Signal<bool>,
    on_click: Callback<web_sys::MouseEvent>,
) -> impl IntoView {
    view! {
        <button
            type="button"
            class="workspace-action-btn"
            prop:disabled=move || actions_disabled.get()
            on:click=move |event| on_click.run(event)
        >
            {move || if is_opening.get() { "Opening…" } else { "New chat" }}
        </button>
    }
}

#[cfg(target_family = "wasm")]
#[component]
fn WorkspaceSelectButton(
    actions_disabled: Signal<bool>,
    is_selected: Signal<bool>,
    on_click: Callback<web_sys::MouseEvent>,
) -> impl IntoView {
    view! {
        <button
            type="button"
            class="workspace-action-btn"
            prop:disabled=move || actions_disabled.get() || is_selected.get()
            on:click=move |event| on_click.run(event)
        >
            {move || workspace_select_button_label(is_selected.get())}
        </button>
    }
}

#[cfg(target_family = "wasm")]
#[component]
fn WorkspaceRenameButton(
    actions_disabled: Signal<bool>,
    on_click: Callback<web_sys::MouseEvent>,
) -> impl IntoView {
    view! {
        <button
            type="button"
            class="workspace-action-btn"
            prop:disabled=move || actions_disabled.get()
            on:click=move |event| on_click.run(event)
        >
            "Rename"
        </button>
    }
}

#[cfg(target_family = "wasm")]
#[component]
fn WorkspaceDeleteButton(
    actions_disabled: Signal<bool>,
    is_deleting: Signal<bool>,
    on_click: Callback<web_sys::MouseEvent>,
) -> impl IntoView {
    view! {
        <button
            type="button"
            class="workspace-action-btn workspace-action-btn--danger"
            prop:disabled=move || actions_disabled.get()
            on:click=move |event| on_click.run(event)
        >
            {move || if is_deleting.get() { "Deleting…" } else { "Delete" }}
        </button>
    }
}

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
                    store_selected_workspace_id(&workspace_id);
                    state.selected_workspace_id.set(Some(workspace_id.clone()));
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
fn workspace_select_handler(
    workspace_id: String,
    workspace_name: String,
    state: WorkspacesPageState,
) -> Callback<web_sys::MouseEvent> {
    Callback::new(move |_| {
        if state.selected_workspace_id.get_untracked().as_deref() == Some(workspace_id.as_str()) {
            return;
        }

        clear_prepared_session_id();
        store_selected_workspace_id(&workspace_id);
        state.selected_workspace_id.set(Some(workspace_id.clone()));
        state.error.set(None);
        state
            .notice
            .set(Some(workspace_selected_notice(&workspace_name)));
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
                    clear_selected_workspace_id_if_matches(&workspace_id);
                    if state.selected_workspace_id.get_untracked().as_deref()
                        == Some(workspace_id.as_str())
                    {
                        state.selected_workspace_id.set(None);
                    }
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

#[component]
#[cfg(not(target_family = "wasm"))]
fn WorkspaceRow(workspace: WorkspaceSummary, state: WorkspacesPageState) -> impl IntoView {
    let display = workspace_row_display(&workspace);
    let row_state = workspace_row_flags(state, &display.workspace_id);
    let draft = state.edit_name_draft.get_untracked();

    workspace_row_view_host(display, draft, row_state)
}

#[cfg(not(target_family = "wasm"))]
fn workspace_row_view_host(
    display: WorkspaceRowDisplay,
    draft: String,
    row_state: WorkspaceRowFlags,
) -> impl IntoView {
    let WorkspaceRowDisplay {
        workspace_name,
        workspace_status,
        created_label,
        ..
    } = display;

    view! {
        <tr>
            <td>
                <WorkspaceNameCellHost
                    workspace_name=workspace_name
                    draft=draft
                    is_editing=row_state.is_editing
                    is_saving=row_state.is_saving
                />
            </td>
            <td>{workspace_status}</td>
            <td>{created_label}</td>
            <td>
                <WorkspaceActionCellHost
                    is_editing=row_state.is_editing
                    is_deleting=row_state.is_deleting
                    is_opening=row_state.is_opening
                    is_selected=row_state.is_selected
                />
            </td>
        </tr>
    }
}

#[component]
#[cfg(not(target_family = "wasm"))]
fn WorkspaceNameCellHost(
    workspace_name: String,
    draft: String,
    is_editing: bool,
    is_saving: bool,
) -> impl IntoView {
    if is_editing {
        view! { <WorkspaceRenameFormHost draft=draft is_saving=is_saving /> }.into_any()
    } else {
        view! { <span>{workspace_name}</span> }.into_any()
    }
}

#[component]
#[cfg(not(target_family = "wasm"))]
fn WorkspaceRenameFormHost(draft: String, is_saving: bool) -> impl IntoView {
    view! {
        <form class="workspace-inline-form">
            <input
                type="text"
                class="workspace-name-input"
                prop:value=draft
                prop:disabled=is_saving
            />
            <button type="submit" class="workspace-action-btn" prop:disabled=is_saving>
                {if is_saving { "Saving…" } else { "Save" }}
            </button>
            <button type="button" class="workspace-action-btn" prop:disabled=is_saving>
                "Cancel"
            </button>
        </form>
    }
}

#[component]
#[cfg(not(target_family = "wasm"))]
fn WorkspaceActionCellHost(
    is_editing: bool,
    is_deleting: bool,
    is_opening: bool,
    is_selected: bool,
) -> impl IntoView {
    if is_editing {
        ().into_any()
    } else {
        view! {
            <WorkspaceActionButtonsHost
                is_deleting=is_deleting
                is_opening=is_opening
                is_selected=is_selected
            />
        }
        .into_any()
    }
}

#[component]
#[cfg(not(target_family = "wasm"))]
fn WorkspaceActionButtonsHost(
    is_deleting: bool,
    is_opening: bool,
    is_selected: bool,
) -> impl IntoView {
    view! {
        <>
            <button
                type="button"
                class="workspace-action-btn"
                prop:disabled=is_deleting || is_opening
            >
                {if is_opening { "Opening…" } else { "New chat" }}
            </button>
            <button
                type="button"
                class="workspace-action-btn"
                prop:disabled=is_deleting || is_opening || is_selected
            >
                {workspace_select_button_label(is_selected)}
            </button>
            <button
                type="button"
                class="workspace-action-btn"
                prop:disabled=is_deleting || is_opening
            >
                "Rename"
            </button>
            <button
                type="button"
                class="workspace-action-btn workspace-action-btn--danger"
                prop:disabled=is_deleting || is_opening
            >
                {if is_deleting { "Deleting…" } else { "Delete" }}
            </button>
        </>
    }
}

fn workspace_select_button_label(is_selected: bool) -> &'static str {
    if is_selected {
        "Selected"
    } else {
        "Switch here"
    }
}

fn workspace_selected_notice(workspace_name: &str) -> String {
    format!("New chats will start in {workspace_name}.")
}

fn workspace_count_label(count: usize) -> String {
    match count {
        0 => "No workspaces".to_string(),
        1 => "1 workspace".to_string(),
        n => format!("{n} workspaces"),
    }
}

fn selected_workspace_name(
    workspaces: &[WorkspaceSummary],
    selected_workspace_id: Option<&str>,
) -> Option<String> {
    let selected_workspace_id = selected_workspace_id?;
    workspaces
        .iter()
        .find(|workspace| workspace.workspace_id == selected_workspace_id)
        .map(|workspace| workspace.name.clone())
}

fn workspace_registry_summary(
    workspaces: &[WorkspaceSummary],
    selected_workspace_id: Option<&str>,
) -> String {
    let count_label = workspace_count_label(workspaces.len());
    if let Some(selected_workspace_name) =
        selected_workspace_name(workspaces, selected_workspace_id)
    {
        format!("{count_label} · Selected: {selected_workspace_name}")
    } else {
        count_label
    }
}

#[cfg(test)]
mod tests {
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

    #[test]
    fn workspace_count_label_pluralises_correctly() {
        assert_eq!(workspace_count_label(0), "No workspaces");
        assert_eq!(workspace_count_label(1), "1 workspace");
        assert_eq!(workspace_count_label(3), "3 workspaces");
    }

    #[test]
    fn workspace_selection_helpers_render_expected_labels() {
        assert_eq!(workspace_select_button_label(false), "Switch here");
        assert_eq!(workspace_select_button_label(true), "Selected");
        assert_eq!(
            workspace_selected_notice("Workspace B"),
            "New chats will start in Workspace B."
        );
    }

    #[test]
    fn workspace_registry_summary_mentions_the_selected_workspace() {
        let workspaces = vec![
            sample_workspace("w_1", "Workspace A"),
            sample_workspace("w_2", "Workspace B"),
        ];

        assert_eq!(
            workspace_registry_summary(&workspaces, Some("w_2")),
            "2 workspaces · Selected: Workspace B"
        );
        assert_eq!(
            workspace_registry_summary(&workspaces, None),
            "2 workspaces"
        );
        assert_eq!(
            workspace_registry_summary(&workspaces, Some("w_missing")),
            "2 workspaces"
        );
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
    fn workspace_row_builds_in_view_mode() {
        let owner = Owner::new();
        owner.with(|| {
            let state = WorkspacesPageState::new();
            let workspace = sample_workspace("w_1", "Test Workspace");
            let _ = view! { <WorkspaceRow workspace=workspace state=state /> };
        });
    }

    #[test]
    fn workspace_row_builds_in_edit_mode() {
        let owner = Owner::new();
        owner.with(|| {
            let state = WorkspacesPageState::new();
            state.editing_workspace_id.set(Some("w_1".to_string()));
            state.edit_name_draft.set("Draft Name".to_string());
            let workspace = sample_workspace("w_1", "Test Workspace");
            let _ = view! { <WorkspaceRow workspace=workspace state=state /> };
        });
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn workspace_row_builds_with_selected_state_on_host() {
        let owner = Owner::new();
        owner.with(|| {
            let state = WorkspacesPageState::new();
            state.selected_workspace_id.set(Some("w_1".to_string()));
            let workspace = sample_workspace("w_1", "Test Workspace");
            let _ = view! { <WorkspaceRow workspace=workspace state=state /> };
        });
    }
}
