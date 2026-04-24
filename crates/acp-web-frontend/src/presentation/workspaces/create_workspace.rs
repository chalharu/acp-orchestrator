#![cfg_attr(not(target_family = "wasm"), allow(dead_code))]

use leptos::prelude::*;

use crate::components::ErrorBanner;
#[cfg(target_family = "wasm")]
use crate::infrastructure::api;

use super::shared::{WorkspacesPageState, spawn_workspace_reload};

/// A button that opens the create-workspace modal when clicked.
#[component]
pub(super) fn CreateWorkspaceButton(state: WorkspacesPageState) -> impl IntoView {
    create_workspace_button(state)
}

#[cfg(target_family = "wasm")]
fn create_workspace_button(state: WorkspacesPageState) -> impl IntoView {
    view! {
        <button
            type="button"
            class="workspace-dashboard__new-btn"
            on:click=move |_| {
                state.error.set(None);
                state.show_create_modal.set(true);
            }
        >
            "+ New workspace"
        </button>
    }
}

#[cfg(not(target_family = "wasm"))]
fn create_workspace_button(state: WorkspacesPageState) -> impl IntoView {
    let _ = state;
    view! {
        <button type="button" class="workspace-dashboard__new-btn">
            "+ New workspace"
        </button>
    }
}

/// The modal overlay + dialog for creating a new workspace. Only rendered when
/// `state.show_create_modal` is `true`.
#[component]
pub(super) fn CreateWorkspaceModal(state: WorkspacesPageState) -> impl IntoView {
    create_workspace_modal(state)
}

#[cfg(target_family = "wasm")]
fn create_workspace_modal(state: WorkspacesPageState) -> impl IntoView {
    let on_submit = create_workspace_submit_handler(state);
    let on_cancel = move |_: web_sys::MouseEvent| {
        state.create_name.set(String::new());
        state.error.set(None);
        state.show_create_modal.set(false);
    };

    view! {
        <Show when=move || state.show_create_modal.get()>
            <div class="workspace-modal-overlay" role="dialog" aria-modal="true" aria-label="Create workspace">
                <div class="workspace-modal">
                    <div class="workspace-modal__header">
                        <h2 class="workspace-modal__title">"Create workspace"</h2>
                        <button
                            type="button"
                            class="workspace-modal__close"
                            on:click=on_cancel
                            aria-label="Close"
                        >
                            "✕"
                        </button>
                    </div>
                    <p class="muted">"Add a new workspace for organising agent sessions."</p>
                    <ErrorBanner message=Signal::derive(move || state.error.get()) />
                    <form class="account-form account-form--create" on:submit=on_submit>
                        <label class="account-form__field">
                            <span>"Name"</span>
                            <input
                                type="text"
                                prop:value=move || state.create_name.get()
                                on:input=move |event| state.create_name.set(event_target_value(&event))
                                autofocus
                            />
                        </label>
                        <div class="workspace-modal__actions">
                            <button
                                type="submit"
                                class="account-form__submit"
                                prop:disabled=move || state.creating.get()
                            >
                                {move || create_workspace_button_label(state.creating.get())}
                            </button>
                            <button
                                type="button"
                                class="account-form__cancel"
                                on:click=on_cancel
                                prop:disabled=move || state.creating.get()
                            >
                                "Cancel"
                            </button>
                        </div>
                    </form>
                </div>
            </div>
        </Show>
    }
}

#[cfg(not(target_family = "wasm"))]
fn create_workspace_modal(state: WorkspacesPageState) -> impl IntoView {
    if !state.show_create_modal.get_untracked() {
        return ().into_any();
    }

    let on_submit = create_workspace_submit_handler(state);
    let creating = state.creating.get_untracked();
    let create_name = state.create_name.get_untracked();

    view! {
        <div class="workspace-modal-overlay" role="dialog" aria-modal="true">
            <div class="workspace-modal">
                <div class="workspace-modal__header">
                    <h2 class="workspace-modal__title">"Create workspace"</h2>
                    <button type="button" class="workspace-modal__close">"✕"</button>
                </div>
                <p class="muted">"Add a new workspace for organising agent sessions."</p>
                <ErrorBanner message=Signal::derive(move || state.error.get()) />
                <form class="account-form account-form--create" on:submit=on_submit>
                    <label class="account-form__field">
                        <span>"Name"</span>
                        <input
                            type="text"
                            prop:value=create_name
                            on:input=move |event| state.create_name.set(event_target_value(&event))
                        />
                    </label>
                    <div class="workspace-modal__actions">
                        <button type="submit" class="account-form__submit" prop:disabled=creating>
                            {create_workspace_button_label(creating)}
                        </button>
                        <button type="button" class="account-form__cancel" prop:disabled=creating>
                            "Cancel"
                        </button>
                    </div>
                </form>
            </div>
        </div>
    }
    .into_any()
}

#[cfg(target_family = "wasm")]
fn create_workspace_submit_handler(
    state: WorkspacesPageState,
) -> impl Fn(web_sys::SubmitEvent) + Copy + 'static {
    move |event: web_sys::SubmitEvent| {
        event.prevent_default();
        if state.creating.get_untracked() {
            return;
        }

        let name = state.create_name.get_untracked();
        if name.trim().is_empty() {
            state
                .error
                .set(Some("Workspace name is required.".to_string()));
            return;
        }

        state.creating.set(true);
        state.error.set(None);
        state.notice.set(None);
        leptos::task::spawn_local(async move {
            match api::create_workspace(&name).await {
                Ok(_) => {
                    state.create_name.set(String::new());
                    state.show_create_modal.set(false);
                    state.notice.set(Some("Workspace created.".to_string()));
                    state.creating.set(false);
                    spawn_workspace_reload(state);
                }
                Err(message) => {
                    state.creating.set(false);
                    state.error.set(Some(message));
                }
            }
        });
    }
}

#[cfg(not(target_family = "wasm"))]
fn create_workspace_submit_handler(
    state: WorkspacesPageState,
) -> impl Fn(web_sys::SubmitEvent) + Copy + 'static {
    move |_event: web_sys::SubmitEvent| create_workspace_submit_host(state)
}

fn create_workspace_button_label(creating: bool) -> &'static str {
    if creating {
        "Saving…"
    } else {
        "Create workspace"
    }
}

#[cfg(not(target_family = "wasm"))]
pub(super) fn create_workspace_submit_host(state: WorkspacesPageState) {
    if state.creating.get_untracked() {
        return;
    }

    state.creating.set(true);
    state.error.set(None);
    state.notice.set(None);
    let _name = state.create_name.get_untracked();
    state.create_name.set(String::new());
    state.show_create_modal.set(false);
    state.notice.set(Some("Workspace created.".to_string()));
    state.creating.set(false);
    spawn_workspace_reload(state);
}

#[cfg(test)]
mod tests {
    use leptos::prelude::*;

    use super::*;
    use crate::presentation::workspaces::shared::WorkspacesPageState;
    use wasm_bindgen::{JsCast, JsValue};

    #[cfg(not(target_family = "wasm"))]
    fn fake_submit_event() -> web_sys::SubmitEvent {
        JsValue::NULL.unchecked_into()
    }

    #[test]
    fn create_workspace_button_label_toggles_with_in_progress_state() {
        assert_eq!(create_workspace_button_label(false), "Create workspace");
        assert_eq!(create_workspace_button_label(true), "Saving…");
    }

    #[test]
    fn create_workspace_button_builds_without_panicking() {
        let owner = Owner::new();
        owner.with(|| {
            let state = WorkspacesPageState::new();
            let _ = view! { <CreateWorkspaceButton state=state /> };
        });
    }

    #[test]
    fn create_workspace_modal_builds_without_panicking() {
        let owner = Owner::new();
        owner.with(|| {
            let state = WorkspacesPageState::new();
            let _ = view! { <CreateWorkspaceModal state=state /> };
        });
    }

    #[test]
    fn create_workspace_modal_builds_when_shown() {
        let owner = Owner::new();
        owner.with(|| {
            let state = WorkspacesPageState::new();
            state.show_create_modal.set(true);
            let _ = view! { <CreateWorkspaceModal state=state /> };
        });
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn create_workspace_submit_host_closes_modal_and_sets_notice() {
        let owner = Owner::new();
        owner.with(|| {
            let state = WorkspacesPageState::new();
            state.create_name.set("My Workspace".to_string());
            state.show_create_modal.set(true);
            create_workspace_submit_host(state);
            assert!(!state.creating.get());
            assert!(state.create_name.get().is_empty());
            assert!(!state.show_create_modal.get());
            assert_eq!(state.notice.get(), Some("Workspace created.".to_string()));
        });
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn host_callback_leaves_in_progress_state_unchanged() {
        let owner = Owner::new();
        owner.with(|| {
            let state = WorkspacesPageState::new();
            state.creating.set(true);
            state.notice.set(Some("still creating".to_string()));
            create_workspace_submit_host(state);
            assert_eq!(state.notice.get(), Some("still creating".to_string()));
            create_workspace_submit_handler(state)(fake_submit_event());
        });
    }
}
