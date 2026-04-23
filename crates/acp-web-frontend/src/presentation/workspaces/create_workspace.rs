#![cfg_attr(not(target_family = "wasm"), allow(dead_code))]

use leptos::prelude::*;

#[cfg(target_family = "wasm")]
use crate::infrastructure::api;

use super::shared::{WorkspacesPageState, spawn_workspace_reload};

#[component]
#[cfg(target_family = "wasm")]
pub(super) fn CreateWorkspaceSection(state: WorkspacesPageState) -> impl IntoView {
    let on_submit = create_workspace_submit_handler(state);

    view! {
        <div class="account-panel__section">
            <div class="account-panel__section-heading">
                <div class="account-panel__section-copy">
                    <h2>"Create workspace"</h2>
                    <p class="muted">"Add a new workspace for organizing agent sessions."</p>
                </div>
            </div>
            <form class="account-form account-form--create" on:submit=on_submit>
                <label class="account-form__field">
                    <span>"Name"</span>
                    <input
                        type="text"
                        prop:value=move || state.create_name.get()
                        on:input=move |event| state.create_name.set(event_target_value(&event))
                    />
                </label>
                <button
                    type="submit"
                    class="account-form__submit"
                    prop:disabled=move || state.creating.get()
                >
                    {move || create_workspace_button_label(state.creating.get())}
                </button>
            </form>
        </div>
    }
}

#[component]
#[cfg(not(target_family = "wasm"))]
pub(super) fn CreateWorkspaceSection(state: WorkspacesPageState) -> impl IntoView {
    let on_submit = create_workspace_submit_handler(state);
    let creating = state.creating.get_untracked();
    let create_name = state.create_name.get_untracked();

    view! {
        <div class="account-panel__section">
            <div class="account-panel__section-heading">
                <div class="account-panel__section-copy">
                    <h2>"Create workspace"</h2>
                    <p class="muted">"Add a new workspace for organizing agent sessions."</p>
                </div>
            </div>
            <form class="account-form account-form--create" on:submit=on_submit>
                <label class="account-form__field">
                    <span>"Name"</span>
                    <input
                        type="text"
                        prop:value=create_name
                        on:input=move |event| state.create_name.set(event_target_value(&event))
                    />
                </label>
                <button type="submit" class="account-form__submit" prop:disabled=creating>
                    {create_workspace_button_label(creating)}
                </button>
            </form>
        </div>
    }
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
    fn create_workspace_section_builds_without_panicking() {
        let owner = Owner::new();
        owner.with(|| {
            let state = WorkspacesPageState::new();
            let _ = view! { <CreateWorkspaceSection state=state /> };
        });
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn create_workspace_submit_host_resets_form_and_sets_notice() {
        let owner = Owner::new();
        owner.with(|| {
            let state = WorkspacesPageState::new();
            state.create_name.set("My Workspace".to_string());
            create_workspace_submit_host(state);
            assert!(!state.creating.get());
            assert!(state.create_name.get().is_empty());
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
