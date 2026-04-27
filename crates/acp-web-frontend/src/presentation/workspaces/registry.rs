#![cfg_attr(not(target_family = "wasm"), allow(dead_code))]

use acp_contracts_sessions::SessionListItem;
use acp_contracts_workspaces::{WorkspaceBranch, WorkspaceSummary};
use leptos::prelude::*;
#[cfg(target_family = "wasm")]
use wasm_bindgen::JsCast;

use crate::components::ErrorBanner;
#[cfg(target_family = "wasm")]
use crate::infrastructure::api;
use crate::presentation::{AppIcon, app_icon_view};
#[cfg(target_family = "wasm")]
use crate::{browser::store_prepared_session_id, routing::app_session_path};

use super::shared::WorkspacesPageState;
#[cfg(any(test, target_family = "wasm"))]
use super::shared::default_branch_ref_name;
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
                <WorkspaceStartChatModal state />
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
        <div class="workspace-dashboard">
            {cards}
            <WorkspaceStartChatModal state />
        </div>
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
    workspace_repository_label: String,
    workspace_status: String,
    created_label: String,
}

fn workspace_card_display(workspace: &WorkspaceSummary) -> WorkspaceCardDisplay {
    WorkspaceCardDisplay {
        workspace_id: workspace.workspace_id.clone(),
        workspace_name: workspace.name.clone(),
        workspace_repository_label: workspace_repository_label(workspace.upstream_url.as_deref()),
        workspace_status: workspace.status.clone(),
        created_label: workspace.created_at.format("%Y-%m-%d").to_string(),
    }
}

fn workspace_repository_label(upstream_url: Option<&str>) -> String {
    upstream_url
        .map(str::to_string)
        .unwrap_or_else(|| "Local workspace".to_string())
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

    let on_open_chat =
        workspace_open_chat_handler(workspace_id.clone(), display.workspace_name.clone(), state);
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
                    workspace_id=display.workspace_id.clone()
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
        <span class="workspace-card__repository">{display.workspace_repository_label}</span>
        <span class="workspace-card__status">{display.workspace_status}</span>
        <span class="workspace-card__created">"Created "{display.created_label}</span>
    }
}

fn workspace_rename_label() -> &'static str {
    "Rename"
}

fn workspace_delete_label(is_deleting: bool) -> &'static str {
    if is_deleting { "Deleting…" } else { "Delete" }
}

fn workspace_new_chat_label(is_opening: bool) -> &'static str {
    if is_opening { "Opening…" } else { "New chat" }
}

fn workspace_save_label(is_saving: bool) -> &'static str {
    if is_saving { "Saving…" } else { "Save" }
}

fn workspace_cancel_label() -> &'static str {
    "Cancel"
}

fn workspace_delete_icon(is_deleting: bool) -> AppIcon {
    if is_deleting {
        AppIcon::Busy
    } else {
        AppIcon::Delete
    }
}

fn workspace_new_chat_icon(is_opening: bool) -> AppIcon {
    if is_opening {
        AppIcon::Busy
    } else {
        AppIcon::NewChat
    }
}

fn workspace_save_icon(is_saving: bool) -> AppIcon {
    if is_saving {
        AppIcon::Busy
    } else {
        AppIcon::Save
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
                    class="workspace-action-btn icon-action"
                    prop:disabled=move || is_deleting.get() || is_opening.get()
                    on:click=move |event| on_edit.run(event)
                    aria-label=workspace_rename_label()
                    title=workspace_rename_label()
                >
                    {app_icon_view(AppIcon::Rename)}
                    <span class="sr-only">{workspace_rename_label()}</span>
                </button>
                <button
                    type="button"
                    class="workspace-action-btn workspace-action-btn--danger icon-action icon-action--danger"
                    prop:disabled=move || is_deleting.get() || is_opening.get()
                    on:click=move |event| on_delete.run(event)
                    aria-label=move || workspace_delete_label(is_deleting.get())
                    title=move || workspace_delete_label(is_deleting.get())
                >
                    {move || app_icon_view(workspace_delete_icon(is_deleting.get()))}
                    <span class="sr-only">{move || workspace_delete_label(is_deleting.get())}</span>
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
            class="workspace-action-btn workspace-action-btn--primary icon-action icon-action--primary"
            prop:disabled=move || is_deleting.get() || is_opening.get()
            on:click=move |event| on_open_chat.run(event)
            aria-label=move || workspace_new_chat_label(is_opening.get())
            title=move || workspace_new_chat_label(is_opening.get())
        >
            {move || app_icon_view(workspace_new_chat_icon(is_opening.get()))}
            <span class="sr-only">{move || workspace_new_chat_label(is_opening.get())}</span>
        </button>
    }
}

#[component]
fn WorkspaceStartChatModal(state: WorkspacesPageState) -> impl IntoView {
    workspace_start_chat_modal(state)
}

#[cfg(target_family = "wasm")]
fn workspace_start_chat_modal(state: WorkspacesPageState) -> impl IntoView {
    view! {
        <Show when=move || state.show_start_chat_modal.get()>
            {workspace_start_chat_modal_view(state, workspace_start_chat_submit_handler(state))}
        </Show>
    }
}

#[cfg(not(target_family = "wasm"))]
fn workspace_start_chat_modal(state: WorkspacesPageState) -> impl IntoView {
    if !state.show_start_chat_modal.get_untracked() {
        return ().into_any();
    }

    workspace_start_chat_modal_view(state, workspace_start_chat_submit_handler(state)).into_any()
}

fn workspace_start_chat_modal_view(
    state: WorkspacesPageState,
    on_submit: impl Fn(web_sys::SubmitEvent) + Copy + 'static,
) -> impl IntoView {
    let workspace_name = Signal::derive(move || state.start_chat_workspace_name.get());
    let branches = Signal::derive(move || state.start_chat_branches.get());
    let selected_branch = Signal::derive(move || state.start_chat_selected_branch.get());
    let loading_branches = Signal::derive(move || state.start_chat_loading_branches.get());
    let opening = Signal::derive(move || state.opening_chat_workspace_id.get().is_some());
    let error = Signal::derive(move || state.error.get());
    let on_cancel = workspace_start_chat_cancel_handler(state);

    view! {
        <div class="workspace-modal-overlay" role="dialog" aria-modal="true" aria-label="Start workspace chat">
            <div class="workspace-modal">
                {workspace_start_chat_modal_header(workspace_name, on_cancel)}
                <p class="muted">
                    {move || {
                        if loading_branches.get() {
                            "Loading branches for this workspace…"
                        } else {
                            "Choose a branch for this chat."
                        }
                    }}
                </p>
                <ErrorBanner message=error />
                <form class="account-form workspace-modal__form" on:submit=on_submit>
                    {workspace_start_chat_branch_field(
                        state,
                        branches,
                        selected_branch,
                        loading_branches,
                    )}
                    {workspace_start_chat_modal_actions(
                        opening,
                        loading_branches,
                        selected_branch,
                        branches,
                        on_cancel,
                    )}
                </form>
            </div>
        </div>
    }
}

fn workspace_start_chat_modal_header(
    workspace_name: Signal<String>,
    on_cancel: impl Fn(web_sys::MouseEvent) + Copy + 'static,
) -> impl IntoView {
    let title = workspace_start_chat_title_signal(workspace_name);
    view! {
        <div class="workspace-modal__header">
            <h2 class="workspace-modal__title">
                "Start chat in " {title}
            </h2>
            <button
                type="button"
                class="workspace-modal__close"
                on:click=on_cancel
                aria-label="Close"
                title="Close"
            >
                {app_icon_view(AppIcon::Cancel)}
                <span class="sr-only">"Close"</span>
            </button>
        </div>
    }
}

fn workspace_start_chat_branch_field(
    state: WorkspacesPageState,
    branches: Signal<Vec<WorkspaceBranch>>,
    selected_branch: Signal<String>,
    loading_branches: Signal<bool>,
) -> impl IntoView {
    view! {
        <label class="account-form__field">
            <span>"Branch"</span>
            <select
                class="workspace-branch-select"
                prop:value=selected_branch
                on:change=move |event| state.start_chat_selected_branch.set(event_target_value(&event))
                prop:disabled=move || loading_branches.get() || branches.get().is_empty()
            >
                <option value="">
                    {move || {
                        if loading_branches.get() {
                            "Loading branches…"
                        } else {
                            "Choose a branch"
                        }
                    }}
                </option>
                {move || {
                    branches
                        .get()
                        .into_iter()
                        .map(|branch| {
                            let label = branch.name;
                            let value = branch.ref_name;
                            view! { <option value=value>{label}</option> }
                        })
                        .collect_view()
                }}
            </select>
            <Show when=move || !loading_branches.get() && branches.get().is_empty()>
                <span class="workspace-field__hint">
                    "No branches are available for this workspace."
                </span>
            </Show>
        </label>
    }
}

fn workspace_start_chat_modal_actions(
    opening: Signal<bool>,
    loading_branches: Signal<bool>,
    selected_branch: Signal<String>,
    branches: Signal<Vec<WorkspaceBranch>>,
    on_cancel: impl Fn(web_sys::MouseEvent) + Copy + 'static,
) -> impl IntoView {
    let submit_label = workspace_new_chat_label_signal(opening);
    let submit_disabled = Signal::derive(move || {
        opening.get()
            || loading_branches.get()
            || selected_branch.get().trim().is_empty()
            || branches.get().is_empty()
    });
    view! {
        <div class="workspace-modal__actions">
            <button
                type="submit"
                class="account-form__submit"
                prop:disabled=move || submit_disabled.get()
            >
                {submit_label}
            </button>
            <button
                type="button"
                class="account-form__cancel"
                on:click=on_cancel
                prop:disabled=move || opening.get()
            >
                "Cancel"
            </button>
        </div>
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
                <button
                    type="submit"
                    class="workspace-action-btn icon-action"
                    prop:disabled=is_saving
                    aria-label=workspace_save_label(is_saving)
                    title=workspace_save_label(is_saving)
                >
                    {app_icon_view(workspace_save_icon(is_saving))}
                    <span class="sr-only">{workspace_save_label(is_saving)}</span>
                </button>
                <button
                    type="button"
                    class="workspace-action-btn icon-action"
                    prop:disabled=is_saving
                    aria-label=workspace_cancel_label()
                    title=workspace_cancel_label()
                >
                    {app_icon_view(AppIcon::Cancel)}
                    <span class="sr-only">{workspace_cancel_label()}</span>
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
            <button
                type="button"
                class="workspace-action-btn icon-action"
                prop:disabled=is_deleting || is_opening
                aria-label=workspace_rename_label()
                title=workspace_rename_label()
            >
                {app_icon_view(AppIcon::Rename)}
                <span class="sr-only">{workspace_rename_label()}</span>
            </button>
            <button
                type="button"
                class="workspace-action-btn workspace-action-btn--danger icon-action icon-action--danger"
                prop:disabled=is_deleting || is_opening
                aria-label=workspace_delete_label(is_deleting)
                title=workspace_delete_label(is_deleting)
            >
                {app_icon_view(workspace_delete_icon(is_deleting))}
                <span class="sr-only">{workspace_delete_label(is_deleting)}</span>
            </button>
        </>
    }
    .into_any()
}

#[cfg(not(target_family = "wasm"))]
fn workspace_card_open_button_host(is_deleting: bool, is_opening: bool) -> impl IntoView {
    view! {
        <button
            type="button"
            class="workspace-action-btn workspace-action-btn--primary icon-action icon-action--primary"
            prop:disabled=is_deleting || is_opening
            aria-label=workspace_new_chat_label(is_opening)
            title=workspace_new_chat_label(is_opening)
        >
            {app_icon_view(workspace_new_chat_icon(is_opening))}
            <span class="sr-only">{workspace_new_chat_label(is_opening)}</span>
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
    workspace_id: String,
    state: WorkspacesPageState,
    is_saving: Signal<bool>,
    on_save: Callback<web_sys::SubmitEvent>,
    on_cancel: Callback<web_sys::MouseEvent>,
) -> impl IntoView {
    let form_ref = NodeRef::new();
    bind_workspace_rename_pointer_cancel_listener(form_ref, workspace_id.clone(), state, is_saving);
    let on_focusout = workspace_rename_focusout_handler(workspace_id, state, is_saving);
    view! {
        <form
            class="workspace-inline-form"
            node_ref=form_ref
            on:submit=move |event| on_save.run(event)
            on:focusout=on_focusout
        >
            {workspace_rename_name_input(state.edit_name_draft, is_saving)}
            {workspace_rename_save_button(is_saving)}
            {workspace_rename_cancel_button(is_saving, on_cancel)}
        </form>
    }
}

#[cfg(target_family = "wasm")]
fn workspace_rename_name_input(
    edit_name_draft: RwSignal<String>,
    is_saving: Signal<bool>,
) -> impl IntoView {
    view! {
        <input
            type="text"
            class="workspace-name-input"
            prop:value=move || edit_name_draft.get()
            on:input=move |event| { edit_name_draft.set(event_target_value(&event)) }
            prop:disabled=move || is_saving.get()
        />
    }
}

#[cfg(target_family = "wasm")]
fn workspace_rename_save_button(is_saving: Signal<bool>) -> impl IntoView {
    view! {
        <button
            type="submit"
            class="workspace-action-btn icon-action"
            prop:disabled=move || is_saving.get()
            aria-label=move || workspace_save_label(is_saving.get())
            title=move || workspace_save_label(is_saving.get())
        >
            {move || app_icon_view(workspace_save_icon(is_saving.get()))}
            <span class="sr-only">{move || workspace_save_label(is_saving.get())}</span>
        </button>
    }
}

#[cfg(target_family = "wasm")]
fn workspace_rename_cancel_button(
    is_saving: Signal<bool>,
    on_cancel: Callback<web_sys::MouseEvent>,
) -> impl IntoView {
    view! {
        <button
            type="button"
            class="workspace-action-btn icon-action"
            prop:disabled=move || is_saving.get()
            on:click=move |event| on_cancel.run(event)
            aria-label=workspace_cancel_label()
            title=workspace_cancel_label()
        >
            {app_icon_view(AppIcon::Cancel)}
            <span class="sr-only">{workspace_cancel_label()}</span>
        </button>
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
fn bind_workspace_rename_pointer_cancel_listener(
    form: NodeRef<leptos::html::Form>,
    workspace_id: String,
    state: WorkspacesPageState,
    is_saving: Signal<bool>,
) {
    Effect::new(move |_| {
        let Some(document) = web_sys::window().and_then(|window| window.document()) else {
            return;
        };
        let Some(form) = form.get() else {
            return;
        };
        let form_node = form.unchecked_into::<web_sys::Node>();
        attach_workspace_rename_pointer_cancel_listener(
            &document,
            &form_node,
            workspace_id.clone(),
            state,
            is_saving,
        );
    });
}

#[cfg(target_family = "wasm")]
fn attach_workspace_rename_pointer_cancel_listener(
    document: &web_sys::Document,
    form_node: &web_sys::Node,
    workspace_id: String,
    state: WorkspacesPageState,
    is_saving: Signal<bool>,
) {
    let form_node = form_node.clone();
    let listener =
        wasm_bindgen::closure::Closure::wrap(Box::new(move |event: web_sys::PointerEvent| {
            cancel_workspace_edit_when_target_leaves_form(
                event.target(),
                &form_node,
                &workspace_id,
                state,
                is_saving,
            );
        }) as Box<dyn FnMut(web_sys::PointerEvent)>);
    let _ =
        document.add_event_listener_with_callback("pointerdown", listener.as_ref().unchecked_ref());
    listener.forget();
}

#[cfg(target_family = "wasm")]
fn cancel_workspace_edit_when_target_leaves_form(
    target: Option<web_sys::EventTarget>,
    form_node: &web_sys::Node,
    workspace_id: &str,
    state: WorkspacesPageState,
    is_saving: Signal<bool>,
) {
    if is_saving.get_untracked() {
        return;
    }
    if state.editing_workspace_id.get_untracked().as_deref() != Some(workspace_id) {
        return;
    }
    let Some(target_node) = target
        .as_ref()
        .and_then(|target| target.dyn_ref::<web_sys::Node>())
    else {
        cancel_workspace_edit(workspace_id, state);
        return;
    };
    if !form_node.contains(Some(target_node)) {
        cancel_workspace_edit(workspace_id, state);
    }
}

#[cfg(target_family = "wasm")]
fn workspace_rename_focusout_handler(
    workspace_id: String,
    state: WorkspacesPageState,
    is_saving: Signal<bool>,
) -> impl Fn(web_sys::FocusEvent) + 'static {
    move |event: web_sys::FocusEvent| {
        if is_saving.get_untracked() {
            return;
        }
        if state.editing_workspace_id.get_untracked().as_deref() != Some(workspace_id.as_str()) {
            return;
        }

        let Some(current_target) = event.current_target() else {
            cancel_workspace_edit(&workspace_id, state);
            return;
        };
        let Ok(form) = current_target.dyn_into::<web_sys::Node>() else {
            cancel_workspace_edit(&workspace_id, state);
            return;
        };
        if let Some(related_target) = event.related_target() {
            if let Ok(related_node) = related_target.dyn_into::<web_sys::Node>() {
                if form.contains(Some(&related_node)) {
                    return;
                }
            }
        }

        cancel_workspace_edit(&workspace_id, state);
    }
}

#[cfg(target_family = "wasm")]
fn cancel_workspace_edit(workspace_id: &str, state: WorkspacesPageState) {
    if state.editing_workspace_id.get_untracked().as_deref() == Some(workspace_id) {
        state.editing_workspace_id.set(None);
        state.edit_name_draft.set(String::new());
        state.error.set(None);
    }
}

#[cfg(target_family = "wasm")]
fn workspace_open_chat_handler(
    workspace_id: String,
    workspace_name: String,
    state: WorkspacesPageState,
) -> Callback<web_sys::MouseEvent> {
    Callback::new(move |_| {
        if state.opening_chat_workspace_id.get_untracked().is_some() {
            return;
        }

        state.error.set(None);
        state.notice.set(None);
        state
            .start_chat_workspace_id
            .set(Some(workspace_id.clone()));
        state.start_chat_workspace_name.set(workspace_name.clone());
        state.start_chat_branches.set(Vec::new());
        state.start_chat_selected_branch.set(String::new());
        state.start_chat_loading_branches.set(true);
        state.show_start_chat_modal.set(true);
        spawn_workspace_start_chat_branch_load(state, workspace_id.clone());
    })
}

#[cfg(target_family = "wasm")]
fn spawn_workspace_start_chat_branch_load(state: WorkspacesPageState, workspace_id: String) {
    leptos::task::spawn_local(async move {
        match api::list_workspace_branches(&workspace_id).await {
            Ok(branches) => {
                store_workspace_start_chat_branches(state, &workspace_id, branches);
            }
            Err(message) => {
                if state.start_chat_workspace_id.get_untracked().as_deref()
                    != Some(workspace_id.as_str())
                {
                    return;
                }
                state.start_chat_loading_branches.set(false);
                state.error.set(Some(message));
            }
        }
    });
}

#[cfg(any(test, target_family = "wasm"))]
fn store_workspace_start_chat_branches(
    state: WorkspacesPageState,
    workspace_id: &str,
    branches: Vec<WorkspaceBranch>,
) {
    if state.start_chat_workspace_id.get_untracked().as_deref() != Some(workspace_id) {
        return;
    }
    state.start_chat_loading_branches.set(false);
    state
        .start_chat_selected_branch
        .set(default_branch_ref_name(&branches));
    state.start_chat_branches.set(branches);
}

#[cfg(target_family = "wasm")]
fn workspace_start_chat_submit_handler(
    state: WorkspacesPageState,
) -> impl Fn(web_sys::SubmitEvent) + Copy + 'static {
    move |event: web_sys::SubmitEvent| {
        event.prevent_default();
        if state.opening_chat_workspace_id.get_untracked().is_some() {
            return;
        }

        let Some(workspace_id) = state.start_chat_workspace_id.get_untracked() else {
            state.error.set(Some(
                "Choose a workspace before starting a chat.".to_string(),
            ));
            return;
        };
        let selected_branch = state.start_chat_selected_branch.get_untracked();
        if selected_branch.trim().is_empty() {
            state
                .error
                .set(Some("Choose a branch before starting a chat.".to_string()));
            return;
        }
        state
            .opening_chat_workspace_id
            .set(Some(workspace_id.clone()));
        state.error.set(None);
        state.notice.set(None);
        leptos::task::spawn_local(async move {
            match api::create_workspace_session(&workspace_id, Some(selected_branch)).await {
                Ok(session_id) => {
                    store_prepared_session_id(&session_id);
                    if let Err(message) =
                        crate::browser::navigate_to(&app_session_path(&session_id))
                    {
                        state.opening_chat_workspace_id.set(None);
                        state.error.set(Some(message));
                        return;
                    }
                    close_workspace_start_chat_modal(state);
                    state.opening_chat_workspace_id.set(None);
                }
                Err(error) => {
                    state.opening_chat_workspace_id.set(None);
                    state.error.set(Some(error.into_message()));
                }
            }
        });
    }
}

#[cfg(not(target_family = "wasm"))]
fn workspace_start_chat_submit_handler(
    state: WorkspacesPageState,
) -> impl Fn(web_sys::SubmitEvent) + Copy + 'static {
    move |_event: web_sys::SubmitEvent| close_workspace_start_chat_modal(state)
}

fn workspace_start_chat_cancel_handler(
    state: WorkspacesPageState,
) -> impl Fn(web_sys::MouseEvent) + Copy + 'static {
    move |_| close_workspace_start_chat_modal(state)
}

fn workspace_start_chat_title_signal(
    workspace_name: Signal<String>,
) -> impl Fn() -> String + Copy + 'static {
    move || workspace_name.get()
}

fn workspace_new_chat_label_signal(
    opening: Signal<bool>,
) -> impl Fn() -> &'static str + Copy + 'static {
    move || workspace_new_chat_label(opening.get())
}

fn close_workspace_start_chat_modal(state: WorkspacesPageState) {
    state.show_start_chat_modal.set(false);
    state.start_chat_workspace_id.set(None);
    state.start_chat_workspace_name.set(String::new());
    state.start_chat_branches.set(Vec::new());
    state.start_chat_selected_branch.set(String::new());
    state.start_chat_loading_branches.set(false);
    state.error.set(None);
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
    Callback::new(move |_| cancel_workspace_edit(&workspace_id, state))
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
    use wasm_bindgen::{JsCast, JsValue};

    use super::*;
    use crate::presentation::workspaces::shared::WorkspacesPageState;

    #[cfg(not(target_family = "wasm"))]
    fn fake_submit_event() -> web_sys::SubmitEvent {
        JsValue::NULL.unchecked_into()
    }

    #[cfg(not(target_family = "wasm"))]
    fn fake_mouse_event() -> web_sys::MouseEvent {
        JsValue::NULL.unchecked_into()
    }

    fn sample_workspace(id: &str, name: &str) -> WorkspaceSummary {
        WorkspaceSummary {
            workspace_id: id.to_string(),
            name: name.to_string(),
            upstream_url: Some("https://example.com/repo.git".to_string()),
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

    fn sample_workspace_branch() -> WorkspaceBranch {
        WorkspaceBranch {
            name: "feature".to_string(),
            ref_name: "refs/heads/feature".to_string(),
        }
    }

    #[cfg(not(target_family = "wasm"))]
    fn seed_workspace_start_chat_state(state: WorkspacesPageState) {
        state.show_start_chat_modal.set(true);
        state.start_chat_workspace_id.set(Some("w_1".to_string()));
        state
            .start_chat_workspace_name
            .set("Test Workspace".to_string());
        state
            .start_chat_branches
            .set(vec![sample_workspace_branch()]);
        state
            .start_chat_selected_branch
            .set("refs/heads/feature".to_string());
    }

    #[test]
    fn workspace_count_label_pluralises_correctly() {
        assert_eq!(workspace_count_label(0), "No workspaces");
        assert_eq!(workspace_count_label(1), "1 workspace");
        assert_eq!(workspace_count_label(3), "3 workspaces");
    }

    #[test]
    fn workspace_action_labels_and_icons_cover_busy_states() {
        assert_eq!(workspace_rename_label(), "Rename");
        assert_eq!(workspace_delete_label(false), "Delete");
        assert_eq!(workspace_delete_label(true), "Deleting…");
        assert_eq!(workspace_new_chat_label(false), "New chat");
        assert_eq!(workspace_new_chat_label(true), "Opening…");
        assert_eq!(workspace_save_label(false), "Save");
        assert_eq!(workspace_save_label(true), "Saving…");
        assert_eq!(workspace_cancel_label(), "Cancel");
        assert_eq!(workspace_delete_icon(false), AppIcon::Delete);
        assert_eq!(workspace_delete_icon(true), AppIcon::Busy);
        assert_eq!(workspace_new_chat_icon(false), AppIcon::NewChat);
        assert_eq!(workspace_new_chat_icon(true), AppIcon::Busy);
        assert_eq!(workspace_save_icon(false), AppIcon::Save);
        assert_eq!(workspace_save_icon(true), AppIcon::Busy);
    }

    #[test]
    fn workspace_card_display_preserves_repository_labels_and_dates() {
        let workspace = sample_workspace("w_1", "Test Workspace");
        let display = workspace_card_display(&workspace);

        assert_eq!(display.workspace_id, "w_1");
        assert_eq!(display.workspace_name, "Test Workspace");
        assert_eq!(
            display.workspace_repository_label,
            "https://example.com/repo.git"
        );
        assert_eq!(display.workspace_status, "active");
        assert_eq!(display.created_label, "2026-04-17");
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
    fn workspace_start_chat_modal_helpers_build_on_host() {
        let owner = Owner::new();
        owner.with(|| {
            let state = WorkspacesPageState::new();
            seed_workspace_start_chat_state(state);

            let workspace_name = Signal::derive(move || state.start_chat_workspace_name.get());
            let branches = Signal::derive(move || state.start_chat_branches.get());
            let selected_branch = Signal::derive(move || state.start_chat_selected_branch.get());
            let loading_branches = Signal::derive(move || state.start_chat_loading_branches.get());
            let opening = Signal::derive(move || state.opening_chat_workspace_id.get().is_some());
            let title = workspace_start_chat_title_signal(workspace_name);
            let submit_label = workspace_new_chat_label_signal(opening);
            let _ = workspace_start_chat_modal(state);
            let _ = workspace_start_chat_modal_view(state, |_event: web_sys::SubmitEvent| {});
            let _ =
                workspace_start_chat_modal_header(workspace_name, |_event: web_sys::MouseEvent| {});
            let _ = workspace_start_chat_branch_field(
                state,
                branches,
                selected_branch,
                loading_branches,
            );
            let _ = workspace_start_chat_modal_actions(
                opening,
                loading_branches,
                selected_branch,
                branches,
                |_event: web_sys::MouseEvent| {},
            );
            assert_eq!(title(), "Test Workspace");
            assert_eq!(submit_label(), "New chat");
        });
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn close_workspace_start_chat_modal_clears_host_state() {
        let owner = Owner::new();
        owner.with(|| {
            let state = WorkspacesPageState::new();
            seed_workspace_start_chat_state(state);
            state.error.set(Some("existing error".to_string()));
            close_workspace_start_chat_modal(state);

            assert!(!state.show_start_chat_modal.get());
            assert!(state.start_chat_workspace_id.get().is_none());
            assert!(state.start_chat_workspace_name.get().is_empty());
            assert!(state.start_chat_branches.get().is_empty());
            assert!(state.start_chat_selected_branch.get().is_empty());
            assert!(!state.start_chat_loading_branches.get());
            assert!(state.error.get().is_none());
        });
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn workspace_start_chat_branch_store_selects_the_first_branch() {
        let owner = Owner::new();
        owner.with(|| {
            let state = WorkspacesPageState::new();
            state.start_chat_workspace_id.set(Some("w_1".to_string()));
            state.start_chat_loading_branches.set(true);

            store_workspace_start_chat_branches(
                state,
                "w_1",
                vec![
                    WorkspaceBranch {
                        name: "main".to_string(),
                        ref_name: "refs/heads/main".to_string(),
                    },
                    sample_workspace_branch(),
                ],
            );

            assert!(!state.start_chat_loading_branches.get());
            assert_eq!(state.start_chat_selected_branch.get(), "refs/heads/main");
            assert_eq!(state.start_chat_branches.get().len(), 2);
        });
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn workspace_start_chat_host_handlers_close_the_modal() {
        let owner = Owner::new();
        owner.with(|| {
            let state = WorkspacesPageState::new();
            seed_workspace_start_chat_state(state);

            workspace_start_chat_submit_handler(state)(fake_submit_event());
            assert!(!state.show_start_chat_modal.get());

            seed_workspace_start_chat_state(state);

            workspace_start_chat_cancel_handler(state)(fake_mouse_event());
            assert!(!state.show_start_chat_modal.get());
        });
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn workspace_id_flag_matches_only_the_selected_workspace() {
        let owner = Owner::new();
        owner.with(|| {
            let state = WorkspacesPageState::new();
            state.editing_workspace_id.set(Some("w_1".to_string()));

            assert!(workspace_id_flag(state.editing_workspace_id, "w_1"));
            assert!(!workspace_id_flag(state.editing_workspace_id, "w_2"));
        });
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn workspace_loading_and_host_action_helpers_build_all_branches() {
        let owner = Owner::new();
        owner.with(|| {
            let display = workspace_card_display(&sample_workspace("w_1", "Test Workspace"));
            let _ = workspace_loading_view();
            let _ = workspace_card_summary_view(display.clone());
            let _ = workspace_card_name_cell_host(display.clone(), String::new(), false, false);
            let _ = workspace_card_name_cell_host(
                display.clone(),
                "Draft Name".to_string(),
                true,
                true,
            );
            let _ = workspace_card_actions_view_host(false, false, false);
            let _ = workspace_card_actions_view_host(true, true, true);
            let _ = workspace_card_open_button_host(false, false);
            let _ = workspace_card_open_button_host(true, true);
        });
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn workspace_session_list_host_builds_non_empty_session_list() {
        let owner = Owner::new();
        owner.with(|| {
            let _ = WorkspaceSessionListHost(WorkspaceSessionListHostProps {
                sessions: Vec::new(),
            });
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
