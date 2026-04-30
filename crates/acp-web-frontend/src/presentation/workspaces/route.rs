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
        state.agent_settings_command.set(opencode_profile_command(
            &state.agent_profiles.get_untracked(),
        ));
        state.show_agent_settings.set(true);
        state.error.set(None);
    }
}

fn opencode_profile_command(profiles: &[acp_contracts_sessions::AgentProfile]) -> String {
    profiles
        .iter()
        .find(|profile| profile.id == "opencode")
        .map(|profile| profile.command_argv.join("\n"))
        .unwrap_or_default()
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
    view! {
        <div class="workspace-modal-overlay" role="dialog" aria-modal="true" aria-label="ACP settings">
            <div class="workspace-modal">
                <div class="workspace-modal__header">
                    <h2 class="workspace-modal__title">"ACP profiles"</h2>
                    <button type="button" class="workspace-modal__close" on:click=on_cancel aria-label="Close" title="Close">
                        {app_icon_view(AppIcon::Cancel)}
                        <span class="sr-only">"Close"</span>
                    </button>
                </div>
                <p class="muted">"Configured profiles are selectable when starting a new chat."</p>
                {agent_profile_list_view(state)}
                <form class="account-form workspace-modal__form" on:submit=on_submit>
                    <label class="account-field">
                        <span>"OpenCode ACP command (one argv per line)"</span>
                        <textarea
                            rows="5"
                            prop:value=move || state.agent_settings_command.get()
                            prop:disabled=!is_admin || state.agent_settings_saving.get()
                            on:input=move |event| state.agent_settings_command.set(event_target_value(&event))
                        />
                    </label>
                    <div class="workspace-modal__actions">
                        <button type="button" class="workspace-action-btn" on:click=on_cancel>"Cancel"</button>
                        <Show when=move || is_admin>
                            <button type="submit" class="workspace-action-btn workspace-action-btn--primary" prop:disabled=move || state.agent_settings_saving.get()>
                                {move || if state.agent_settings_saving.get() { "Saving…" } else { "Save profile" }}
                            </button>
                        </Show>
                    </div>
                </form>
            </div>
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
        .map(|profile| view! { <p class="muted">{profile.name}</p> })
        .collect_view()
        .into_any()
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
        let command = state.agent_settings_command.get_untracked();
        leptos::task::spawn_local(async move {
            match crate::infrastructure::api::save_opencode_agent_profile(command).await {
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

    use super::*;

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
    fn opencode_profile_command_reads_configured_argv_lines() {
        let profiles = vec![acp_contracts_sessions::AgentProfile {
            id: "opencode".to_string(),
            name: "OpenCode ACP".to_string(),
            mode: acp_contracts_sessions::AgentProfileMode::Chroot,
            command_argv: vec!["opencode".to_string(), "acp".to_string()],
            env_allowlist: Vec::new(),
            timeout_seconds: 30,
            run_uid: 65_534,
            run_gid: 65_534,
        }];

        assert_eq!(opencode_profile_command(&profiles), "opencode\nacp");
        assert!(opencode_profile_command(&[]).is_empty());
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
