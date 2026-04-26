#![cfg_attr(not(target_family = "wasm"), allow(dead_code))]

use leptos::prelude::*;

use crate::components::ErrorBanner;
#[cfg(target_family = "wasm")]
use crate::infrastructure::api;
use crate::presentation::{AppIcon, app_icon_view};

use super::shared::{WorkspacesPageState, spawn_workspace_reload};

/// A button that opens the create-workspace modal when clicked.
#[component]
pub(super) fn CreateWorkspaceButton(state: WorkspacesPageState) -> impl IntoView {
    create_workspace_button_view(create_workspace_button_click_handler(state))
}

#[cfg(target_family = "wasm")]
fn create_workspace_button_click_handler(
    state: WorkspacesPageState,
) -> impl Fn(web_sys::MouseEvent) + Copy + 'static {
    move |_| {
        state.error.set(None);
        state.show_create_modal.set(true);
    }
}

#[cfg(not(target_family = "wasm"))]
fn create_workspace_button_click_handler(
    _state: WorkspacesPageState,
) -> impl Fn(web_sys::MouseEvent) + Copy + 'static {
    move |_| {}
}

fn create_workspace_button_view(
    on_click: impl Fn(web_sys::MouseEvent) + Copy + 'static,
) -> impl IntoView {
    view! {
        <button
            type="button"
            class="workspace-dashboard__new-btn"
            on:click=on_click
            aria-label=create_workspace_trigger_label()
            title=create_workspace_trigger_label()
        >
            <span class="workspace-dashboard__new-btn-icon" aria-hidden="true">
                {app_icon_view(AppIcon::CreateWorkspace)}
            </span>
            <span class="workspace-dashboard__new-btn-label">
                {create_workspace_trigger_label()}
            </span>
        </button>
    }
}

fn create_workspace_trigger_label() -> &'static str {
    "New workspace"
}

/// The modal overlay + dialog for creating a new workspace. Only rendered when
/// `state.show_create_modal` is `true`.
#[component]
pub(super) fn CreateWorkspaceModal(state: WorkspacesPageState) -> impl IntoView {
    create_workspace_modal(state)
}

#[cfg(target_family = "wasm")]
fn create_workspace_modal(state: WorkspacesPageState) -> impl IntoView {
    view! {
        <Show when=move || state.show_create_modal.get()>
            {create_workspace_modal_view(state, create_workspace_submit_handler(state))}
        </Show>
    }
}

#[cfg(not(target_family = "wasm"))]
fn create_workspace_modal(state: WorkspacesPageState) -> impl IntoView {
    if !state.show_create_modal.get_untracked() {
        return ().into_any();
    }

    create_workspace_modal_view(state, create_workspace_submit_handler(state)).into_any()
}

fn create_workspace_modal_view(
    state: WorkspacesPageState,
    on_submit: impl Fn(web_sys::SubmitEvent) + Copy + 'static,
) -> impl IntoView {
    let on_cancel = create_workspace_modal_cancel_handler(state);
    let creating = Signal::derive(move || state.creating.get());
    let create_name = Signal::derive(move || state.create_name.get());
    let create_upstream_url = Signal::derive(move || state.create_upstream_url.get());
    let error = Signal::derive(move || state.error.get());

    view! {
        <div class="workspace-modal-overlay" role="dialog" aria-modal="true" aria-label="Create workspace">
            <div class="workspace-modal">
                {create_workspace_modal_header(on_cancel)}
                <p class="muted">"Add a new workspace for organising agent sessions."</p>
                <ErrorBanner message=error />
                <form class="account-form workspace-modal__form" on:submit=on_submit>
                    {create_workspace_name_field(state, create_name)}
                    {create_workspace_upstream_field(state, create_upstream_url)}
                    {create_workspace_modal_actions(creating, on_cancel)}
                </form>
            </div>
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
        let upstream_url = state.create_upstream_url.get_untracked();
        if name.trim().is_empty() {
            state
                .error
                .set(Some("Workspace name is required.".to_string()));
            return;
        }
        if upstream_url.trim().is_empty() {
            state
                .error
                .set(Some("Repository URL is required.".to_string()));
            return;
        }

        state.creating.set(true);
        state.error.set(None);
        state.notice.set(None);
        leptos::task::spawn_local(async move {
            match api::create_workspace(&name, upstream_url).await {
                Ok(_) => {
                    close_create_workspace_modal(state);
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

fn create_workspace_button_label_signal(creating: Signal<bool>) -> Signal<&'static str> {
    Signal::derive(move || create_workspace_button_label(creating.get()))
}

fn create_workspace_modal_header(
    on_cancel: impl Fn(web_sys::MouseEvent) + Copy + 'static,
) -> impl IntoView {
    view! {
        <div class="workspace-modal__header">
            <h2 class="workspace-modal__title">"Create workspace"</h2>
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

fn create_workspace_name_field(
    state: WorkspacesPageState,
    create_name: Signal<String>,
) -> impl IntoView {
    view! {
        <label class="account-form__field">
            <span>"Name"</span>
            <input
                type="text"
                prop:value=create_name
                on:input=move |event| state.create_name.set(event_target_value(&event))
                autofocus
                required
            />
        </label>
    }
}

fn create_workspace_upstream_field(
    state: WorkspacesPageState,
    create_upstream_url: Signal<String>,
) -> impl IntoView {
    view! {
        <label class="account-form__field">
            <span>"Repository URL"</span>
            <input
                type="url"
                prop:value=create_upstream_url
                on:input=move |event| state.create_upstream_url.set(event_target_value(&event))
                placeholder="https://example.com/repo.git"
                required
            />
        </label>
    }
}

fn create_workspace_modal_actions(
    creating: Signal<bool>,
    on_cancel: impl Fn(web_sys::MouseEvent) + Copy + 'static,
) -> impl IntoView {
    let label = create_workspace_button_label_signal(creating);

    view! {
        <div class="workspace-modal__actions">
            <button
                type="submit"
                class="account-form__submit"
                prop:disabled=move || creating.get()
            >
                {label}
            </button>
            <button
                type="button"
                class="account-form__cancel"
                on:click=on_cancel
                prop:disabled=move || creating.get()
            >
                "Cancel"
            </button>
        </div>
    }
}

fn create_workspace_modal_cancel_handler(
    state: WorkspacesPageState,
) -> impl Fn(web_sys::MouseEvent) + Copy + 'static {
    move |_: web_sys::MouseEvent| close_create_workspace_modal(state)
}

fn close_create_workspace_modal(state: WorkspacesPageState) {
    state.create_name.set(String::new());
    state.create_upstream_url.set(String::new());
    state.error.set(None);
    state.show_create_modal.set(false);
}

#[cfg(not(target_family = "wasm"))]
pub(super) fn create_workspace_submit_host(state: WorkspacesPageState) {
    if state.creating.get_untracked() {
        return;
    }

    if state.create_name.get_untracked().trim().is_empty() {
        state
            .error
            .set(Some("Workspace name is required.".to_string()));
        return;
    }
    if state.create_upstream_url.get_untracked().trim().is_empty() {
        state
            .error
            .set(Some("Repository URL is required.".to_string()));
        return;
    }

    state.creating.set(true);
    state.error.set(None);
    state.notice.set(None);
    let _name = state.create_name.get_untracked();
    let _upstream_url = state.create_upstream_url.get_untracked();
    close_create_workspace_modal(state);
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

    #[cfg(not(target_family = "wasm"))]
    fn fake_mouse_event() -> web_sys::MouseEvent {
        JsValue::NULL.unchecked_into()
    }

    #[test]
    fn create_workspace_button_label_toggles_with_in_progress_state() {
        assert_eq!(create_workspace_button_label(false), "Create workspace");
        assert_eq!(create_workspace_button_label(true), "Saving…");
    }

    #[test]
    fn create_workspace_trigger_label_is_stable() {
        assert_eq!(create_workspace_trigger_label(), "New workspace");
    }

    #[test]
    fn create_workspace_button_label_signal_tracks_creating_state() {
        let owner = Owner::new();
        owner.with(|| {
            let creating = RwSignal::new(false);
            let label =
                create_workspace_button_label_signal(Signal::derive(move || creating.get()));

            assert_eq!(label.get(), "Create workspace");
            creating.set(true);
            assert_eq!(label.get(), "Saving…");
        });
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
            state
                .create_upstream_url
                .set("https://example.com/repo.git".to_string());
            state.show_create_modal.set(true);
            create_workspace_submit_host(state);
            assert!(!state.creating.get());
            assert!(state.create_name.get().is_empty());
            assert!(state.create_upstream_url.get().is_empty());
            assert!(!state.show_create_modal.get());
            assert_eq!(state.notice.get(), Some("Workspace created.".to_string()));
        });
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn close_create_workspace_modal_clears_form_state() {
        let owner = Owner::new();
        owner.with(|| {
            let state = WorkspacesPageState::new();
            state.create_name.set("My Workspace".to_string());
            state
                .create_upstream_url
                .set("https://example.com/repo.git".to_string());
            state.error.set(Some("Create failed".to_string()));
            state.show_create_modal.set(true);

            close_create_workspace_modal(state);

            assert!(state.create_name.get().is_empty());
            assert!(state.create_upstream_url.get().is_empty());
            assert!(state.error.get().is_none());
            assert!(!state.show_create_modal.get());
        });
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn host_button_click_handler_is_a_noop() {
        let owner = Owner::new();
        owner.with(|| {
            let state = WorkspacesPageState::new();
            state.error.set(Some("keep".to_string()));
            let handler = create_workspace_button_click_handler(state);

            handler(fake_mouse_event());

            assert_eq!(state.error.get(), Some("keep".to_string()));
            assert!(!state.show_create_modal.get());
        });
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn modal_cancel_handler_closes_modal() {
        let owner = Owner::new();
        owner.with(|| {
            let state = WorkspacesPageState::new();
            state.create_name.set("Draft".to_string());
            state.error.set(Some("Create failed".to_string()));
            state.show_create_modal.set(true);
            let handler = create_workspace_modal_cancel_handler(state);

            handler(fake_mouse_event());

            assert!(state.create_name.get().is_empty());
            assert!(state.error.get().is_none());
            assert!(!state.show_create_modal.get());
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
