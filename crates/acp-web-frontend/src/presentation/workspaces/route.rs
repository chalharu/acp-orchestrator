use leptos::prelude::*;

use crate::presentation::{AppIcon, app_icon_view};
use crate::{application::auth::WorkspacesRouteAccess, components::ErrorBanner};

#[cfg(target_family = "wasm")]
use super::shared::workspaces_back_to_chat_path_from_location;
use super::{
    create_workspace::{CreateWorkspaceButton, CreateWorkspaceModal},
    registry::workspace_registry_section,
    shared::{WorkspacesPageState, initialize_workspaces_page},
};

const AGENT_SETTINGS_FIELD_CLASS: &str = "account-form__field";
const AGENT_SETTINGS_PROFILE_NAME_PLACEHOLDER: &str = "Claude ACP";
const AGENT_SETTINGS_COMMAND_PLACEHOLDER: &str = "claude acp --port ${ACP_PORT}";

#[component]
pub fn WorkspacesPage() -> impl IntoView {
    let state = WorkspacesPageState::new();
    initialize_workspaces_page(state);

    workspaces_page_shell(state)
}

#[cfg(target_family = "wasm")]
fn workspaces_page_shell(state: WorkspacesPageState) -> impl IntoView {
    let back_to_chat_path = workspaces_back_to_chat_path_from_location();
    view! {
        <main class="app-shell account-shell">
            <ErrorBanner message=state.error />
            <section class="panel account-panel">
                <div class="account-panel__header">
                    <h1>"Workspaces"</h1>
                    <div class="account-panel__header-actions">
                        {workspaces_back_link_view(back_to_chat_path)}
                        {agent_settings_button(state)}
                        <CreateWorkspaceButton state />
                    </div>
                </div>
                <Show when=move || state.notice.get().is_some()>
                    <p class="account-notice" role="status">
                        {move || state.notice.get().unwrap_or_default()}
                    </p>
                </Show>
                <WorkspacesPageContent state />
                <CreateWorkspaceModal state />
                {agent_settings_modal(state)}
            </section>
        </main>
    }
}

#[cfg(not(target_family = "wasm"))]
fn workspaces_notice_view(notice: Option<String>) -> AnyView {
    if let Some(notice) = notice {
        view! {
            <p class="account-notice" role="status">
                {notice}
            </p>
        }
        .into_any()
    } else {
        ().into_any()
    }
}

#[cfg(not(target_family = "wasm"))]
fn workspaces_page_shell(state: WorkspacesPageState) -> impl IntoView {
    let notice_view = workspaces_notice_view(state.notice.get_untracked());

    view! {
        <main class="app-shell account-shell">
            <ErrorBanner message=state.error />
            <section class="panel account-panel">
                <div class="account-panel__header">
                    <h1>"Workspaces"</h1>
                    <div class="account-panel__header-actions">
                        {workspaces_back_link_view(None)}
                        {agent_settings_button(state)}
                        <CreateWorkspaceButton state />
                    </div>
                </div>
                {notice_view}
                <WorkspacesPageContent state />
                <CreateWorkspaceModal state />
                {agent_settings_modal(state)}
            </section>
        </main>
    }
}

fn agent_settings_button(state: WorkspacesPageState) -> AnyView {
    let on_click = agent_settings_open_handler(state);
    view! {
        <button
            type="button"
            class="account-panel__header-action icon-action icon-action--ghost"
            on:click=on_click
            aria-label="ACP settings"
            title="ACP settings"
        >
            {app_icon_view(AppIcon::Accounts)}
            <span class="sr-only">"ACP settings"</span>
        </button>
    }
    .into_any()
}

fn agent_settings_open_handler(
    state: WorkspacesPageState,
) -> impl Fn(web_sys::MouseEvent) + Copy + 'static {
    move |_| {
        reset_agent_settings_form(state);
        state.show_agent_settings.set(true);
        state.error.set(None);
    }
}

fn reset_agent_settings_form(state: WorkspacesPageState) {
    state.agent_settings_profile_name.set(String::new());
    state.agent_settings_command.set(String::new());
}

#[cfg(target_family = "wasm")]
fn agent_settings_modal(state: WorkspacesPageState) -> AnyView {
    view! {
        <Show when=move || state.show_agent_settings.get()>
            {agent_settings_modal_view(state)}
        </Show>
    }
    .into_any()
}

#[cfg(not(target_family = "wasm"))]
fn agent_settings_modal(state: WorkspacesPageState) -> AnyView {
    if !state.show_agent_settings.get_untracked() {
        return ().into_any();
    }
    agent_settings_modal_view(state).into_any()
}

fn agent_settings_modal_view(state: WorkspacesPageState) -> impl IntoView {
    let is_admin = agent_settings_is_admin(state);
    let on_cancel = move |_event: web_sys::MouseEvent| state.show_agent_settings.set(false);
    let on_submit = agent_settings_submit_handler(state, is_admin);
    let error = Signal::derive(move || state.error.get());
    view! {
        <div class="workspace-modal-overlay" role="dialog" aria-modal="true" aria-label="ACP settings">
            <div class="workspace-modal">
                {agent_settings_header_view(on_cancel)}
                <p class="muted">"Configured profiles are selectable when starting a new chat."</p>
                {agent_profile_list_view(state)}
                <ErrorBanner message=error />
                {agent_settings_form_view(state, is_admin, on_submit, on_cancel)}
            </div>
        </div>
    }
}

fn agent_settings_header_view(
    on_cancel: impl Fn(web_sys::MouseEvent) + Copy + 'static,
) -> impl IntoView {
    view! {
        <div class="workspace-modal__header">
            <h2 class="workspace-modal__title">"ACP profiles"</h2>
            <button type="button" class="workspace-modal__close" on:click=on_cancel aria-label="Close" title="Close">
                {app_icon_view(AppIcon::Cancel)}
                <span class="sr-only">"Close"</span>
            </button>
        </div>
    }
}

fn agent_settings_form_view(
    state: WorkspacesPageState,
    is_admin: bool,
    on_submit: impl Fn(web_sys::SubmitEvent) + Copy + 'static,
    on_cancel: impl Fn(web_sys::MouseEvent) + Copy + 'static,
) -> impl IntoView {
    view! {
        <form class="account-form workspace-modal__form" on:submit=on_submit>
            {agent_settings_profile_name_field(state, is_admin)}
            {agent_settings_command_field(state, is_admin)}
            {agent_settings_help_view()}
            {agent_settings_actions_view(state, is_admin, on_cancel)}
        </form>
    }
}

fn agent_settings_profile_name_field(state: WorkspacesPageState, is_admin: bool) -> impl IntoView {
    view! {
        <label class=AGENT_SETTINGS_FIELD_CLASS>
            <span>"Profile name"</span>
            <input
                type="text"
                placeholder=AGENT_SETTINGS_PROFILE_NAME_PLACEHOLDER
                prop:value=move || state.agent_settings_profile_name.get()
                prop:disabled=move || !is_admin || state.agent_settings_saving.get()
                on:input=move |event| state.agent_settings_profile_name.set(event_target_value(&event))
            />
        </label>
    }
}

fn agent_settings_command_field(state: WorkspacesPageState, is_admin: bool) -> impl IntoView {
    view! {
        <label class=AGENT_SETTINGS_FIELD_CLASS>
            <span>"ACP launch command"</span>
            <textarea
                rows="3"
                placeholder=AGENT_SETTINGS_COMMAND_PLACEHOLDER
                prop:value=move || state.agent_settings_command.get()
                prop:disabled=move || !is_admin || state.agent_settings_saving.get()
                on:input=move |event| state.agent_settings_command.set(event_target_value(&event))
            />
        </label>
    }
}

fn agent_settings_help_view() -> impl IntoView {
    view! {
        <p class="muted">
            "Profile names must be unique. "
            "Enter a single command line, for example "
            <code>"claude acp --port ${ACP_PORT}"</code>
            ". Quotes and backslash escapes are supported. The backend runs argv directly without a shell, and replaces the "
            <code>"${ACP_PORT}"</code>
            " placeholder at launch."
        </p>
    }
}

fn agent_settings_actions_view(
    state: WorkspacesPageState,
    is_admin: bool,
    on_cancel: impl Fn(web_sys::MouseEvent) + Copy + 'static,
) -> impl IntoView {
    view! {
        <div class="workspace-modal__actions">
            <button type="button" class="workspace-action-btn" on:click=on_cancel>"Cancel"</button>
            <button type="submit" class="workspace-action-btn workspace-action-btn--primary" prop:disabled=move || !is_admin || state.agent_settings_saving.get()>
                "Add profile"
            </button>
        </div>
    }
}

fn agent_profile_list_view(state: WorkspacesPageState) -> AnyView {
    let profiles = state.agent_profiles.get_untracked();
    if profiles.is_empty() {
        return view! { <p class="muted">"No ACP profiles configured."</p> }.into_any();
    }
    profiles
        .into_iter()
        .map(|profile| {
            let command_preview = agent_command_preview(&profile.command_argv);
            view! {
                <p class="muted">
                    <strong>{profile.name}</strong>
                    " "
                    <code>{command_preview}</code>
                </p>
            }
        })
        .collect_view()
        .into_any()
}

fn agent_command_preview(command_argv: &[String]) -> String {
    command_argv
        .iter()
        .map(|arg| preview_argv_arg(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

fn preview_argv_arg(arg: &str) -> String {
    if arg.is_empty() {
        return "''".to_string();
    }
    if arg.chars().all(preview_arg_can_stay_unquoted) {
        arg.to_string()
    } else {
        format!("'{}'", arg.replace('\'', r#"'\''"#))
    }
}

fn preview_arg_can_stay_unquoted(ch: char) -> bool {
    !ch.is_whitespace() && ch != '\'' && ch != '"' && ch != '\\'
}

fn agent_settings_is_admin(state: WorkspacesPageState) -> bool {
    matches!(
        state.access.get_untracked(),
        Some(WorkspacesRouteAccess::SignedIn(account)) if account.is_admin
    )
}

#[cfg(target_family = "wasm")]
fn agent_settings_submit_handler(
    state: WorkspacesPageState,
    is_admin: bool,
) -> impl Fn(web_sys::SubmitEvent) + Copy + 'static {
    move |event| {
        event.prevent_default();
        if !is_admin || state.agent_settings_saving.get_untracked() {
            return;
        }
        state.agent_settings_saving.set(true);
        state.error.set(None);
        let name = state.agent_settings_profile_name.get_untracked();
        let command = state.agent_settings_command.get_untracked();
        leptos::task::spawn_local(async move {
            match crate::infrastructure::api::create_agent_profile(name, command).await {
                Ok(profile) => {
                    state.agent_profiles.update(|profiles| {
                        profiles.retain(|existing| existing.id != profile.id);
                        profiles.push(profile);
                    });
                    state.agent_settings_saving.set(false);
                    state.show_agent_settings.set(false);
                    state.notice.set(Some("ACP profile saved.".to_string()));
                }
                Err(message) => {
                    state.agent_settings_saving.set(false);
                    state.error.set(Some(message));
                }
            }
        });
    }
}

#[cfg(not(target_family = "wasm"))]
fn agent_settings_submit_handler(
    state: WorkspacesPageState,
    _is_admin: bool,
) -> impl Fn(web_sys::SubmitEvent) + Copy + 'static {
    move |_event| state.show_agent_settings.set(false)
}

fn workspaces_back_link_view(back_to_chat_path: Option<String>) -> AnyView {
    back_to_chat_path
        .map(|href| {
            view! {
                <a
                    href=href
                    class="account-panel__header-action icon-action icon-action--ghost"
                    aria-label="Back to chat"
                    title="Back to chat"
                >
                    {app_icon_view(AppIcon::BackToChat)}
                    <span class="sr-only">"Back to chat"</span>
                </a>
            }
            .into_any()
        })
        .unwrap_or_else(|| ().into_any())
}

#[component]
fn WorkspacesPageContent(state: WorkspacesPageState) -> impl IntoView {
    workspaces_page_content(state)
}

#[cfg(target_family = "wasm")]
fn workspaces_page_content(state: WorkspacesPageState) -> impl IntoView {
    move || workspaces_page_content_body(state.access.get(), state)
}

#[cfg(not(target_family = "wasm"))]
fn workspaces_page_content(state: WorkspacesPageState) -> impl IntoView {
    workspaces_page_content_body(state.access.get_untracked(), state)
}

fn workspaces_page_content_body(
    access: Option<WorkspacesRouteAccess>,
    state: WorkspacesPageState,
) -> AnyView {
    match access {
        Some(WorkspacesRouteAccess::SignedIn(_)) => view! {
            {workspace_registry_section(state)}
        }
        .into_any(),
        Some(WorkspacesRouteAccess::RegisterRequired) => view! {
            <p class="muted">
                "Bootstrap registration is still required. "
                <a href="/app/register/">"Create the first account."</a>
            </p>
        }
        .into_any(),
        Some(WorkspacesRouteAccess::SignInRequired) => view! {
            <p class="muted">
                "Sign in is required before managing workspaces. "
                <a href="/app/sign-in/">"Open sign-in."</a>
            </p>
        }
        .into_any(),
        None => view! { <p class="muted">"Checking access…"</p> }.into_any(),
    }
}

#[cfg(test)]
mod tests {
    use acp_contracts_accounts::LocalAccount;
    use chrono::{TimeZone, Utc};
    use leptos::prelude::*;
    use wasm_bindgen::{JsCast, JsValue};

    use super::*;

    fn fake_mouse_event() -> web_sys::MouseEvent {
        JsValue::NULL.unchecked_into()
    }

    fn fake_submit_event() -> web_sys::SubmitEvent {
        JsValue::NULL.unchecked_into()
    }

    #[test]
    fn workspaces_page_content_builds_for_each_access_state() {
        let owner = Owner::new();
        owner.with(|| {
            let state = WorkspacesPageState::new();

            state
                .access
                .set(Some(WorkspacesRouteAccess::SignedIn(sample_account(false))));
            let _ = view! { <WorkspacesPageContent state=state /> };

            state
                .access
                .set(Some(WorkspacesRouteAccess::RegisterRequired));
            let _ = view! { <WorkspacesPageContent state=state /> };

            state
                .access
                .set(Some(WorkspacesRouteAccess::SignInRequired));
            let _ = view! { <WorkspacesPageContent state=state /> };
        });
    }

    #[test]
    fn workspaces_page_and_shell_render_host_safe_views() {
        let owner = Owner::new();
        owner.with(|| {
            let state = WorkspacesPageState::new();
            state.notice.set(Some("Workspace updated.".to_string()));
            state
                .access
                .set(Some(WorkspacesRouteAccess::SignedIn(sample_account(false))));
            let _ = workspaces_page_shell(state);
            let _ = workspaces_back_link_view(Some("/app/sessions/abc".to_string()));
            let _ = view! { <WorkspacesPage /> };
        });
    }

    #[test]
    fn workspaces_page_header_includes_create_button() {
        // Verify that the page shell builds without panicking when the create
        // modal trigger button is present in the header.
        let owner = Owner::new();
        owner.with(|| {
            let state = WorkspacesPageState::new();
            state
                .access
                .set(Some(WorkspacesRouteAccess::SignedIn(sample_account(false))));
            // Modal is not shown by default.
            assert!(!state.show_create_modal.get());
            let _ = workspaces_page_shell(state);
        });
    }

    #[test]
    fn agent_settings_helpers_build_for_admin_and_member() {
        let owner = Owner::new();
        owner.with(|| {
            let state = WorkspacesPageState::new();
            state
                .access
                .set(Some(WorkspacesRouteAccess::SignedIn(sample_account(true))));
            state.show_agent_settings.set(true);
            assert!(agent_settings_is_admin(state));
            let _ = agent_settings_button(state);
            let _ = agent_settings_modal(state);
            let _ = agent_settings_modal_view(state);

            state
                .access
                .set(Some(WorkspacesRouteAccess::SignedIn(sample_account(false))));
            assert!(!agent_settings_is_admin(state));
        });
    }

    #[test]
    fn agent_settings_open_resets_add_profile_form() {
        let owner = Owner::new();
        owner.with(|| {
            let state = WorkspacesPageState::new();
            state
                .agent_settings_profile_name
                .set("Old profile".to_string());
            state.agent_settings_command.set("opencode acp".to_string());

            reset_agent_settings_form(state);

            assert!(state.agent_settings_profile_name.get().is_empty());
            assert!(state.agent_settings_command.get().is_empty());
        });
    }

    #[test]
    fn agent_settings_open_handler_shows_blank_add_form() {
        let owner = Owner::new();
        owner.with(|| {
            let state = WorkspacesPageState::new();
            state.error.set(Some("old error".to_string()));
            state.agent_settings_command.set("opencode acp".to_string());

            agent_settings_open_handler(state)(fake_mouse_event());

            assert!(state.show_agent_settings.get());
            assert!(state.error.get().is_none());
            assert!(state.agent_settings_command.get().is_empty());
        });
    }

    #[test]
    fn agent_settings_modal_uses_form_fields_inline_errors_and_consistent_examples() {
        let owner = Owner::new();
        owner.with(|| {
            let state = WorkspacesPageState::new();
            state
                .access
                .set(Some(WorkspacesRouteAccess::SignedIn(sample_account(true))));
            state
                .error
                .set(Some("Profile name is required".to_string()));

            let _ = agent_settings_modal_view(state);
            assert_eq!(AGENT_SETTINGS_FIELD_CLASS, "account-form__field");
            assert_eq!(AGENT_SETTINGS_PROFILE_NAME_PLACEHOLDER, "Claude ACP");
            assert_eq!(
                AGENT_SETTINGS_COMMAND_PLACEHOLDER,
                "claude acp --port ${ACP_PORT}"
            );
            assert!(!AGENT_SETTINGS_COMMAND_PLACEHOLDER.contains("opencode"));
        });
    }

    #[test]
    fn host_agent_settings_submit_handler_closes_modal() {
        let owner = Owner::new();
        owner.with(|| {
            let state = WorkspacesPageState::new();
            state.show_agent_settings.set(true);

            agent_settings_submit_handler(state, false)(fake_submit_event());

            assert!(!state.show_agent_settings.get());
        });
    }

    #[test]
    fn agent_profile_list_view_builds_command_previews() {
        let owner = Owner::new();
        owner.with(|| {
            let state = WorkspacesPageState::new();
            state.agent_profiles.set(vec![sample_agent_profile(
                "claude",
                "Claude ACP",
                vec![
                    "claude".to_string(),
                    "acp".to_string(),
                    "--config".to_string(),
                    "~/Library/Application Support/Claude/config.json".to_string(),
                ],
            )]);

            let _ = agent_profile_list_view(state);
        });
    }

    #[test]
    fn agent_command_preview_preserves_argv_boundaries() {
        assert_eq!(
            agent_command_preview(&[
                "claude".to_string(),
                "acp".to_string(),
                "--config".to_string(),
                "~/Library/Application Support/Claude/config.json".to_string(),
            ]),
            "claude acp --config '~/Library/Application Support/Claude/config.json'"
        );
        assert_eq!(
            agent_command_preview(&[
                "agent".to_string(),
                "can't".to_string(),
                r#"dir\name"#.to_string(),
                String::new(),
            ]),
            r#"agent 'can'\''t' 'dir\name' ''"#
        );
    }

    fn sample_agent_profile(
        id: &str,
        name: &str,
        command_argv: Vec<String>,
    ) -> acp_contracts_sessions::AgentProfile {
        acp_contracts_sessions::AgentProfile {
            id: id.to_string(),
            name: name.to_string(),
            mode: acp_contracts_sessions::AgentProfileMode::Chroot,
            command_argv,
            env_allowlist: Vec::new(),
            timeout_seconds: 30,
            run_uid: 65_534,
            run_gid: 65_534,
        }
    }

    fn sample_account(is_admin: bool) -> LocalAccount {
        LocalAccount {
            user_id: "u_test".to_string(),
            username: "tester".to_string(),
            is_admin,
            created_at: Utc.with_ymd_and_hms(2026, 4, 17, 1, 0, 0).unwrap(),
        }
    }
}
