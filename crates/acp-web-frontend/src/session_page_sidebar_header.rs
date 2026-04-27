#[cfg(target_family = "wasm")]
use crate::components::{
    ErrorBanner, workspace_branch_modal_actions, workspace_branch_select_field,
    workspace_branch_status_message,
};
#[cfg(any(test, target_family = "wasm"))]
use acp_contracts_workspaces::WorkspaceBranch;
use leptos::prelude::*;

#[cfg(any(test, target_family = "wasm"))]
use crate::presentation::default_branch_ref_name;
#[cfg(target_family = "wasm")]
use crate::{
    browser::{navigate_to, store_prepared_session_id},
    infrastructure::api,
};
use crate::{
    presentation::{
        AppIcon, SessionSidebarAuthControls, app_icon_view, workspaces_path_with_return_to,
    },
    routing::app_session_path,
};

#[cfg(target_family = "wasm")]
#[component]
pub(super) fn SessionSidebarHeader(
    current_session_id: String,
    current_workspace_id: Signal<Option<String>>,
    auth_error: RwSignal<Option<String>>,
    sidebar_error: RwSignal<Option<String>>,
    sidebar_open: RwSignal<bool>,
) -> impl IntoView {
    session_sidebar_header_view(
        current_session_id,
        current_workspace_id,
        auth_error,
        sidebar_error,
        view! {
            <button
                type="button"
                class="session-sidebar__dismiss"
                on:click=move |_| sidebar_open.set(false)
            >
                <span aria-hidden="true">{"✕"}</span>
                <span class="sr-only">"Close sidebar"</span>
            </button>
        },
    )
}

#[cfg(not(target_family = "wasm"))]
#[component]
pub(super) fn SessionSidebarHeader(
    current_session_id: String,
    current_workspace_id: Signal<Option<String>>,
    auth_error: RwSignal<Option<String>>,
    sidebar_error: RwSignal<Option<String>>,
    sidebar_open: RwSignal<bool>,
) -> impl IntoView {
    let _ = sidebar_open;
    session_sidebar_header_view(
        current_session_id,
        current_workspace_id,
        auth_error,
        sidebar_error,
        view! {
            <button type="button" class="session-sidebar__dismiss">
                <span aria-hidden="true">{"✕"}</span>
                <span class="sr-only">"Close sidebar"</span>
            </button>
        },
    )
}

fn session_sidebar_header_view(
    current_session_id: String,
    current_workspace_id: Signal<Option<String>>,
    auth_error: RwSignal<Option<String>>,
    sidebar_error: RwSignal<Option<String>>,
    dismiss_button: impl IntoView + 'static,
) -> impl IntoView {
    let workspaces_href = session_sidebar_workspaces_href(&current_session_id);
    let current_session_id_for_auth = current_session_id.clone();

    #[cfg(target_family = "wasm")]
    let primary_action =
        session_sidebar_primary_action(current_workspace_id, sidebar_error, workspaces_href);
    #[cfg(not(target_family = "wasm"))]
    let primary_action = {
        let _ = sidebar_error;
        session_sidebar_primary_action(current_workspace_id, workspaces_href)
    };

    view! {
        <div class="session-sidebar__header">
            <div class="session-sidebar__header-links">
                {primary_action}
                <SessionSidebarAuthControls current_session_id=current_session_id_for_auth error=auth_error />
            </div>
            {dismiss_button}
        </div>
    }
}

fn session_sidebar_new_chat_workspace_id(current_workspace_id: Option<String>) -> Option<String> {
    current_workspace_id.filter(|workspace_id| !workspace_id.trim().is_empty())
}

fn session_sidebar_new_chat_icon(creating: bool) -> AppIcon {
    if creating {
        AppIcon::Busy
    } else {
        AppIcon::NewChat
    }
}

fn session_sidebar_new_chat_label(creating: bool) -> &'static str {
    if creating { "Creating…" } else { "New chat" }
}

#[cfg(any(test, target_family = "wasm"))]
fn session_sidebar_branch_required_message() -> &'static str {
    "Choose a branch before starting a chat."
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SessionSidebarPrimaryActionKind {
    NewChat,
    Workspaces,
}

fn session_sidebar_primary_action_kind(
    current_workspace_id: Option<String>,
) -> SessionSidebarPrimaryActionKind {
    if session_sidebar_new_chat_workspace_id(current_workspace_id).is_some() {
        SessionSidebarPrimaryActionKind::NewChat
    } else {
        SessionSidebarPrimaryActionKind::Workspaces
    }
}

#[cfg(any(test, target_family = "wasm"))]
#[derive(Clone, Copy)]
struct SessionSidebarNewChatState {
    show_modal: RwSignal<bool>,
    workspace_id: RwSignal<Option<String>>,
    branches: RwSignal<Vec<WorkspaceBranch>>,
    selected_branch: RwSignal<String>,
    loading_branches: RwSignal<bool>,
    creating: RwSignal<bool>,
}

#[cfg(any(test, target_family = "wasm"))]
impl SessionSidebarNewChatState {
    fn new() -> Self {
        Self {
            show_modal: RwSignal::new(false),
            workspace_id: RwSignal::new(None::<String>),
            branches: RwSignal::new(Vec::<WorkspaceBranch>::new()),
            selected_branch: RwSignal::new(String::new()),
            loading_branches: RwSignal::new(false),
            creating: RwSignal::new(false),
        }
    }
}

#[cfg(any(test, target_family = "wasm"))]
fn session_sidebar_new_chat_unavailable_message() -> &'static str {
    "Current workspace is unavailable. Open Workspaces to choose another workspace."
}

#[cfg(any(test, target_family = "wasm"))]
fn session_sidebar_begin_new_chat(
    current_workspace_id: Option<String>,
    sidebar_error: RwSignal<Option<String>>,
    state: SessionSidebarNewChatState,
) -> Option<String> {
    let Some(workspace_id) = session_sidebar_new_chat_workspace_id(current_workspace_id) else {
        sidebar_error.set(Some(
            session_sidebar_new_chat_unavailable_message().to_string(),
        ));
        return None;
    };

    state.show_modal.set(true);
    state.workspace_id.set(Some(workspace_id.clone()));
    state.branches.set(Vec::new());
    state.selected_branch.set(String::new());
    state.loading_branches.set(true);
    state.creating.set(false);
    sidebar_error.set(None);
    Some(workspace_id)
}

#[cfg(any(test, target_family = "wasm"))]
fn session_sidebar_finish_new_chat_failure(
    state: SessionSidebarNewChatState,
    sidebar_error: RwSignal<Option<String>>,
    message: String,
) {
    state.creating.set(false);
    sidebar_error.set(Some(message));
}

#[cfg(any(test, target_family = "wasm"))]
fn session_sidebar_complete_branch_load(
    state: SessionSidebarNewChatState,
    workspace_id: &str,
    branches: Vec<WorkspaceBranch>,
) {
    if state.workspace_id.get_untracked().as_deref() != Some(workspace_id) {
        return;
    }
    state.loading_branches.set(false);
    state
        .selected_branch
        .set(default_branch_ref_name(&branches));
    state.branches.set(branches);
}

#[cfg(target_family = "wasm")]
fn session_sidebar_finish_branch_load_failure(
    state: SessionSidebarNewChatState,
    sidebar_error: RwSignal<Option<String>>,
    workspace_id: &str,
    message: String,
) {
    if state.workspace_id.get_untracked().as_deref() != Some(workspace_id) {
        return;
    }
    state.loading_branches.set(false);
    sidebar_error.set(Some(message));
}

#[cfg(any(test, target_family = "wasm"))]
fn session_sidebar_close_new_chat_modal(
    state: SessionSidebarNewChatState,
    sidebar_error: RwSignal<Option<String>>,
) {
    state.show_modal.set(false);
    state.workspace_id.set(None);
    state.branches.set(Vec::new());
    state.selected_branch.set(String::new());
    state.loading_branches.set(false);
    state.creating.set(false);
    sidebar_error.set(None);
}

fn session_sidebar_workspaces_icon() -> AppIcon {
    AppIcon::Workspaces
}

fn session_sidebar_workspaces_label() -> &'static str {
    "Workspaces"
}

fn session_sidebar_workspaces_href(current_session_id: &str) -> String {
    workspaces_path_with_return_to(&app_session_path(current_session_id))
}

fn session_sidebar_workspaces_link(href: &str) -> AnyView {
    view! {
        <a
            class="session-sidebar__new-link icon-action icon-action--primary"
            href=href.to_string()
            aria-label=session_sidebar_workspaces_label()
            title=session_sidebar_workspaces_label()
        >
            <span class="session-sidebar__new-link-icon" aria-hidden="true">
                {app_icon_view(session_sidebar_workspaces_icon())}
            </span>
            <span class="sr-only">{session_sidebar_workspaces_label()}</span>
        </a>
    }
    .into_any()
}

#[cfg(target_family = "wasm")]
fn session_sidebar_primary_action(
    current_workspace_id: Signal<Option<String>>,
    sidebar_error: RwSignal<Option<String>>,
    workspaces_href: String,
) -> AnyView {
    let action_kind =
        Signal::derive(move || session_sidebar_primary_action_kind(current_workspace_id.get()));

    view! {
        {move || {
            match action_kind.get() {
                SessionSidebarPrimaryActionKind::NewChat => {
                    view! {
                        <SessionSidebarNewChatAction
                            current_workspace_id=current_workspace_id
                            sidebar_error=sidebar_error
                        />
                    }
                    .into_any()
                }
                SessionSidebarPrimaryActionKind::Workspaces => {
                    session_sidebar_workspaces_link(&workspaces_href)
                }
            }
        }}
    }
    .into_any()
}

#[cfg(not(target_family = "wasm"))]
fn session_sidebar_primary_action(
    current_workspace_id: Signal<Option<String>>,
    workspaces_href: String,
) -> AnyView {
    match session_sidebar_primary_action_kind(current_workspace_id.get_untracked()) {
        SessionSidebarPrimaryActionKind::NewChat => {
            session_sidebar_new_chat_button(current_workspace_id)
        }
        SessionSidebarPrimaryActionKind::Workspaces => {
            session_sidebar_workspaces_link(&workspaces_href)
        }
    }
}

#[cfg(target_family = "wasm")]
#[component]
fn SessionSidebarNewChatAction(
    current_workspace_id: Signal<Option<String>>,
    sidebar_error: RwSignal<Option<String>>,
) -> impl IntoView {
    let state = SessionSidebarNewChatState::new();
    let on_click =
        session_sidebar_new_chat_click_handler(current_workspace_id, sidebar_error, state);
    let creating = Signal::derive(move || state.creating.get());

    view! {
        <>
            {session_sidebar_new_chat_button_view(creating, on_click)}
            {session_sidebar_new_chat_modal(state, sidebar_error)}
        </>
    }
}

#[cfg(target_family = "wasm")]
fn session_sidebar_new_chat_button_view(
    creating: Signal<bool>,
    on_click: Callback<web_sys::MouseEvent>,
) -> impl IntoView {
    view! {
        <button
            type="button"
            class="session-sidebar__new-link icon-action icon-action--primary"
            prop:disabled=move || creating.get()
            aria-label=move || session_sidebar_new_chat_label(creating.get())
            title=move || session_sidebar_new_chat_label(creating.get())
            on:click=move |event| on_click.run(event)
        >
            <span class="session-sidebar__new-link-icon" aria-hidden="true">
                {move || app_icon_view(session_sidebar_new_chat_icon(creating.get()))}
            </span>
            <span class="sr-only">{move || session_sidebar_new_chat_label(creating.get())}</span>
        </button>
    }
}

#[cfg(target_family = "wasm")]
fn session_sidebar_new_chat_modal(
    state: SessionSidebarNewChatState,
    sidebar_error: RwSignal<Option<String>>,
) -> impl IntoView {
    let error = Signal::derive(move || sidebar_error.get());
    let on_submit = session_sidebar_new_chat_submit_handler(sidebar_error, state);
    let on_cancel = session_sidebar_new_chat_cancel_handler(sidebar_error, state);

    view! {
        <Show when=move || state.show_modal.get()>
            {session_sidebar_new_chat_modal_view(state, error, on_submit, on_cancel)}
        </Show>
    }
}

#[cfg(target_family = "wasm")]
fn session_sidebar_new_chat_modal_view(
    state: SessionSidebarNewChatState,
    error: Signal<Option<String>>,
    on_submit: Callback<web_sys::SubmitEvent>,
    on_cancel: Callback<web_sys::MouseEvent>,
) -> impl IntoView {
    let branches = Signal::derive(move || state.branches.get());
    let selected_branch = Signal::derive(move || state.selected_branch.get());
    let loading_branches = Signal::derive(move || state.loading_branches.get());
    let creating = Signal::derive(move || state.creating.get());

    view! {
        <div
            class="workspace-modal-overlay"
            role="dialog"
            aria-modal="true"
            aria-label="Start new chat"
        >
            <div class="workspace-modal">
                {session_sidebar_new_chat_modal_header(on_cancel)}
                <p class="muted">
                    {move || workspace_branch_status_message(loading_branches.get())}
                </p>
                <ErrorBanner message=error />
                <form
                    class="account-form workspace-modal__form"
                    on:submit=move |event| on_submit.run(event)
                >
                    {session_sidebar_new_chat_branch_field(
                        state,
                        branches,
                        selected_branch,
                        loading_branches,
                    )}
                    {session_sidebar_new_chat_modal_actions(
                        creating,
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

#[cfg(target_family = "wasm")]
fn session_sidebar_new_chat_modal_header(
    on_cancel: Callback<web_sys::MouseEvent>,
) -> impl IntoView {
    view! {
        <div class="workspace-modal__header">
            <h2 class="workspace-modal__title">"Start new chat"</h2>
            <button
                type="button"
                class="workspace-modal__close"
                on:click=move |event| on_cancel.run(event)
                aria-label="Close"
                title="Close"
            >
                {app_icon_view(AppIcon::Cancel)}
                <span class="sr-only">"Close"</span>
            </button>
        </div>
    }
}

#[cfg(target_family = "wasm")]
fn session_sidebar_new_chat_click_handler(
    current_workspace_id: Signal<Option<String>>,
    sidebar_error: RwSignal<Option<String>>,
    state: SessionSidebarNewChatState,
) -> Callback<web_sys::MouseEvent> {
    Callback::new(move |_| {
        if state.creating.get_untracked() {
            return;
        }

        let Some(workspace_id) = session_sidebar_begin_new_chat(
            current_workspace_id.get_untracked(),
            sidebar_error,
            state,
        ) else {
            return;
        };

        session_sidebar_spawn_branch_request(workspace_id, sidebar_error, state);
    })
}

#[cfg(target_family = "wasm")]
fn session_sidebar_spawn_branch_request(
    workspace_id: String,
    sidebar_error: RwSignal<Option<String>>,
    state: SessionSidebarNewChatState,
) {
    leptos::task::spawn_local(async move {
        match api::list_workspace_branches(&workspace_id).await {
            Ok(branches) => {
                session_sidebar_complete_branch_load(state, &workspace_id, branches);
            }
            Err(message) => {
                session_sidebar_finish_branch_load_failure(
                    state,
                    sidebar_error,
                    &workspace_id,
                    message,
                );
            }
        }
    });
}

#[cfg(target_family = "wasm")]
fn session_sidebar_new_chat_branch_field(
    state: SessionSidebarNewChatState,
    branches: Signal<Vec<WorkspaceBranch>>,
    selected_branch: Signal<String>,
    loading_branches: Signal<bool>,
) -> impl IntoView {
    workspace_branch_select_field(branches, selected_branch, loading_branches, move |event| {
        state.selected_branch.set(event_target_value(&event))
    })
}

#[cfg(target_family = "wasm")]
fn session_sidebar_new_chat_modal_actions(
    creating: Signal<bool>,
    loading_branches: Signal<bool>,
    selected_branch: Signal<String>,
    branches: Signal<Vec<WorkspaceBranch>>,
    on_cancel: Callback<web_sys::MouseEvent>,
) -> impl IntoView {
    workspace_branch_modal_actions(
        move || session_sidebar_new_chat_label(creating.get()),
        creating,
        loading_branches,
        selected_branch,
        branches,
        move |event| on_cancel.run(event),
    )
}

#[cfg(target_family = "wasm")]
fn session_sidebar_new_chat_submit_handler(
    sidebar_error: RwSignal<Option<String>>,
    state: SessionSidebarNewChatState,
) -> Callback<web_sys::SubmitEvent> {
    Callback::new(move |event: web_sys::SubmitEvent| {
        event.prevent_default();
        if state.creating.get_untracked() || state.loading_branches.get_untracked() {
            return;
        }

        let Some(workspace_id) = state.workspace_id.get_untracked() else {
            sidebar_error.set(Some(
                session_sidebar_new_chat_unavailable_message().to_string(),
            ));
            return;
        };
        let selected_branch = state.selected_branch.get_untracked();
        if selected_branch.trim().is_empty() {
            sidebar_error.set(Some(session_sidebar_branch_required_message().to_string()));
            return;
        }

        state.creating.set(true);
        sidebar_error.set(None);
        leptos::task::spawn_local(async move {
            match api::create_workspace_session(&workspace_id, Some(selected_branch)).await {
                Ok(session_id) => {
                    store_prepared_session_id(&session_id);
                    if let Err(message) = navigate_to(&app_session_path(&session_id)) {
                        session_sidebar_finish_new_chat_failure(state, sidebar_error, message);
                        return;
                    }
                    session_sidebar_close_new_chat_modal(state, sidebar_error);
                }
                Err(create_error) => {
                    session_sidebar_finish_new_chat_failure(
                        state,
                        sidebar_error,
                        create_error.into_message(),
                    );
                }
            }
        });
    })
}

#[cfg(target_family = "wasm")]
fn session_sidebar_new_chat_cancel_handler(
    sidebar_error: RwSignal<Option<String>>,
    state: SessionSidebarNewChatState,
) -> Callback<web_sys::MouseEvent> {
    Callback::new(move |_| session_sidebar_close_new_chat_modal(state, sidebar_error))
}

#[cfg(not(target_family = "wasm"))]
fn session_sidebar_new_chat_button(current_workspace_id: Signal<Option<String>>) -> AnyView {
    let can_create =
        session_sidebar_new_chat_workspace_id(current_workspace_id.get_untracked()).is_some();
    let label = session_sidebar_new_chat_label(false);

    view! {
        <button
            type="button"
            class="session-sidebar__new-link icon-action icon-action--primary"
            prop:disabled=!can_create
            aria-label=label
            title=label
        >
            <span class="session-sidebar__new-link-icon" aria-hidden="true">
                {app_icon_view(session_sidebar_new_chat_icon(false))}
            </span>
            <span class="sr-only">{label}</span>
        </button>
    }
    .into_any()
}

#[cfg(test)]
mod tests {
    use leptos::prelude::*;

    use super::*;

    fn sample_sidebar_branch() -> WorkspaceBranch {
        WorkspaceBranch {
            name: "main".to_string(),
            ref_name: "refs/heads/main".to_string(),
        }
    }

    #[test]
    fn session_sidebar_header_builds_without_panicking() {
        let owner = Owner::new();
        owner.with(|| {
            let auth_error = RwSignal::new(None::<String>);
            let sidebar_error = RwSignal::new(None::<String>);
            let sidebar_open = RwSignal::new(true);
            let current_workspace_id = Signal::derive(|| Some("workspace-1".to_string()));
            let _ = view! {
                <SessionSidebarHeader
                    current_session_id="session-1".to_string()
                    current_workspace_id=current_workspace_id
                    auth_error=auth_error
                    sidebar_error=sidebar_error
                    sidebar_open=sidebar_open
                />
            };
        });
    }

    #[test]
    fn session_sidebar_header_builds_without_workspace_context() {
        let owner = Owner::new();
        owner.with(|| {
            let auth_error = RwSignal::new(None::<String>);
            let sidebar_error = RwSignal::new(None::<String>);
            let sidebar_open = RwSignal::new(true);
            let current_workspace_id = Signal::derive(|| None::<String>);
            let _ = view! {
                <SessionSidebarHeader
                    current_session_id="session-1".to_string()
                    current_workspace_id=current_workspace_id
                    auth_error=auth_error
                    sidebar_error=sidebar_error
                    sidebar_open=sidebar_open
                />
            };
        });
    }

    #[test]
    fn session_sidebar_new_chat_helpers_cover_ready_and_busy_states() {
        assert_eq!(
            session_sidebar_new_chat_workspace_id(Some("workspace-1".to_string())),
            Some("workspace-1".to_string())
        );
        assert_eq!(
            session_sidebar_new_chat_workspace_id(Some("   ".to_string())),
            None
        );
        assert_eq!(session_sidebar_new_chat_workspace_id(None), None);
        assert_eq!(
            session_sidebar_primary_action_kind(Some("workspace-1".to_string())),
            SessionSidebarPrimaryActionKind::NewChat
        );
        assert_eq!(
            session_sidebar_primary_action_kind(None),
            SessionSidebarPrimaryActionKind::Workspaces
        );
        assert_eq!(session_sidebar_new_chat_icon(false), AppIcon::NewChat);
        assert_eq!(session_sidebar_new_chat_icon(true), AppIcon::Busy);
        assert_eq!(session_sidebar_new_chat_label(false), "New chat");
        assert_eq!(session_sidebar_new_chat_label(true), "Creating…");
        assert_eq!(
            session_sidebar_branch_required_message(),
            "Choose a branch before starting a chat."
        );
        assert_eq!(
            session_sidebar_new_chat_unavailable_message(),
            "Current workspace is unavailable. Open Workspaces to choose another workspace."
        );
        assert_eq!(session_sidebar_workspaces_icon(), AppIcon::Workspaces);
        assert_eq!(session_sidebar_workspaces_label(), "Workspaces");
        assert_eq!(
            session_sidebar_workspaces_href("session-1"),
            "/app/workspaces/?return_to=%2Fapp%2Fsessions%2Fsession-1"
        );
    }

    #[test]
    fn session_sidebar_begin_new_chat_sets_modal_state() {
        let owner = Owner::new();
        owner.with(|| {
            let sidebar_error = RwSignal::new(Some("old".to_string()));
            let state = SessionSidebarNewChatState::new();

            assert_eq!(
                session_sidebar_begin_new_chat(
                    Some("workspace-1".to_string()),
                    sidebar_error,
                    state,
                ),
                Some("workspace-1".to_string())
            );
            assert!(state.show_modal.get());
            assert_eq!(state.workspace_id.get(), Some("workspace-1".to_string()));
            assert!(state.loading_branches.get());
            assert_eq!(sidebar_error.get(), None);

            session_sidebar_complete_branch_load(
                state,
                "workspace-1",
                vec![sample_sidebar_branch()],
            );
            assert!(!state.loading_branches.get());
            assert_eq!(state.selected_branch.get(), "refs/heads/main");
            assert_eq!(state.branches.get().len(), 1);
        });
    }

    #[test]
    fn session_sidebar_branch_load_ignores_stale_workspace_updates() {
        let owner = Owner::new();
        owner.with(|| {
            let sidebar_error = RwSignal::new(None::<String>);
            let state = SessionSidebarNewChatState::new();
            let _ = session_sidebar_begin_new_chat(
                Some("workspace-1".to_string()),
                sidebar_error,
                state,
            );

            session_sidebar_complete_branch_load(
                state,
                "workspace-2",
                vec![sample_sidebar_branch()],
            );

            assert!(state.loading_branches.get());
            assert!(state.selected_branch.get().is_empty());
            assert!(state.branches.get().is_empty());
        });
    }

    #[test]
    fn session_sidebar_begin_new_chat_reports_missing_workspace() {
        let owner = Owner::new();
        owner.with(|| {
            let sidebar_error = RwSignal::new(Some("old".to_string()));
            let state = SessionSidebarNewChatState::new();
            assert_eq!(
                session_sidebar_begin_new_chat(None, sidebar_error, state),
                None
            );
            assert_eq!(
                sidebar_error.get(),
                Some(session_sidebar_new_chat_unavailable_message().to_string())
            );
        });
    }

    #[test]
    fn session_sidebar_finish_new_chat_failure_and_close_modal_reset_state() {
        let owner = Owner::new();
        owner.with(|| {
            let sidebar_error = RwSignal::new(None::<String>);
            let state = SessionSidebarNewChatState::new();
            let _ = session_sidebar_begin_new_chat(
                Some("workspace-1".to_string()),
                sidebar_error,
                state,
            );
            session_sidebar_complete_branch_load(
                state,
                "workspace-1",
                vec![sample_sidebar_branch()],
            );
            session_sidebar_finish_new_chat_failure(
                state,
                sidebar_error,
                "unable to create".to_string(),
            );
            assert!(!state.creating.get());
            assert_eq!(sidebar_error.get(), Some("unable to create".to_string()));

            session_sidebar_close_new_chat_modal(state, sidebar_error);
            assert!(!state.show_modal.get());
            assert!(state.workspace_id.get().is_none());
            assert!(state.branches.get().is_empty());
            assert!(state.selected_branch.get().is_empty());
            assert!(!state.loading_branches.get());
            assert!(sidebar_error.get().is_none());
        });
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn host_sidebar_primary_actions_render_for_new_chat_and_workspaces() {
        let owner = Owner::new();
        owner.with(|| {
            let _ = session_sidebar_workspaces_link("/app/workspaces/");
            let _ = session_sidebar_primary_action(
                Signal::derive(|| Some("workspace-1".to_string())),
                "/app/workspaces/".to_string(),
            );
            let _ = session_sidebar_primary_action(
                Signal::derive(|| None::<String>),
                "/app/workspaces/".to_string(),
            );
            let _ =
                session_sidebar_new_chat_button(Signal::derive(|| Some("workspace-1".to_string())));
            let _ = session_sidebar_new_chat_button(Signal::derive(|| None::<String>));
        });
    }
}
