#![cfg_attr(not(target_family = "wasm"), allow(dead_code))]

use acp_contracts_sessions::{AgentProfile, AgentProfileMode};
use leptos::prelude::*;

#[cfg(target_family = "wasm")]
use crate::infrastructure::api;
use crate::presentation::{AppIcon, app_icon_view};

#[cfg(target_family = "wasm")]
use super::shared::spawn_agent_profiles_reload;
use super::shared::AccountsPageState;

const PROFILE_COMMAND_PLACEHOLDER: &str = "opencode acp --host 127.0.0.1 --port ${ACP_PORT}";

#[component]
pub(super) fn AgentProfilesSection(state: AccountsPageState) -> impl IntoView {
    view! {
        <div class="account-panel__section account-panel__section--registry">
            <div class="account-panel__section-heading">
                <div class="account-panel__section-copy">
                    <h2>"ACP profiles"</h2>
                    <p class="muted">
                        "Configure local ACP launch commands. Profiles are selectable when starting a new chat."
                    </p>
                </div>
            </div>
            {agent_profile_create_form(state)}
            {agent_profile_table(state)}
        </div>
    }
}

fn agent_profile_create_form(state: AccountsPageState) -> AnyView {
    let on_submit = create_agent_profile_submit_handler(state);
    view! {
        <form class="account-form account-form--create agent-profile-form" on:submit=on_submit>
            <label class="account-form__field">
                <span>"Profile name"</span>
                <input
                    type="text"
                    placeholder="OpenCode ACP"
                    prop:value=move || state.agent_profile_name.get()
                    on:input=move |event| state.agent_profile_name.set(event_target_value(&event))
                />
            </label>
            <label class="account-form__field agent-profile-form__command">
                <span>"ACP launch command"</span>
                <textarea
                    rows="3"
                    placeholder=PROFILE_COMMAND_PLACEHOLDER
                    prop:value=move || state.agent_profile_command.get()
                    on:input=move |event| state.agent_profile_command.set(event_target_value(&event))
                />
            </label>
            <label class="account-form__field">
                <span>"Launch mode"</span>
                <select
                    prop:value=move || profile_mode_value(state.agent_profile_mode.get())
                    on:change=move |event| state.agent_profile_mode.set(profile_mode_from_value(&event_target_value(&event)))
                >
                    <option value="host">"Host process"</option>
                    <option value="chroot">"Chroot isolation"</option>
                </select>
            </label>
            <button
                type="submit"
                class="account-form__submit"
                prop:disabled=move || state.agent_profile_saving.get()
            >
                {move || agent_profile_save_label(state.agent_profile_saving.get())}
            </button>
        </form>
        <p class="muted">
            "Host process is the default for rootless local ACP servers. Use chroot only when the host is privileged and configured for isolation. "
            <code>"${ACP_PORT}"</code>
            " and related ACP placeholders are expanded by the backend."
        </p>
    }
    .into_any()
}

#[cfg(target_family = "wasm")]
fn agent_profile_table(state: AccountsPageState) -> AnyView {
    view! {
        <Show
            when=move || !state.agent_profiles_loading.get()
            fallback=|| view! { <p class="muted">"Loading ACP profiles…"</p> }
        >
            {move || agent_profile_table_body(state)}
        </Show>
    }
    .into_any()
}

#[cfg(not(target_family = "wasm"))]
fn agent_profile_table(state: AccountsPageState) -> AnyView {
    if state.agent_profiles_loading.get_untracked() {
        return view! { <p class="muted">"Loading ACP profiles…"</p> }.into_any();
    }
    agent_profile_table_body(state)
}

fn agent_profile_table_body(state: AccountsPageState) -> AnyView {
    let profiles = state.agent_profiles.get();
    if profiles.is_empty() {
        return view! { <p class="muted">"No ACP profiles configured."</p> }.into_any();
    }
    let rows = profiles
        .into_iter()
        .map(|profile| agent_profile_row(profile, state))
        .collect_view();
    view! {
        <div class="account-table-wrap">
            <table class="account-table agent-profile-table">
                <caption class="sr-only">"ACP profile commands and launch modes"</caption>
                <thead>
                    <tr>
                        <th scope="col">"Profile"</th>
                        <th scope="col">"Command"</th>
                        <th scope="col">"Mode"</th>
                        <th scope="col">"Actions"</th>
                    </tr>
                </thead>
                <tbody>{rows}</tbody>
            </table>
        </div>
    }
    .into_any()
}

fn agent_profile_row(profile: AgentProfile, state: AccountsPageState) -> AnyView {
    let name = RwSignal::new(profile.name.clone());
    let command = RwSignal::new(agent_command_preview(&profile.command_argv));
    let mode = RwSignal::new(profile.mode.clone());
    let saving = RwSignal::new(false);
    let deleting = Signal::derive({
        let profile_id = profile.id.clone();
        move || state.deleting_agent_profile_id.get().as_deref() == Some(profile_id.as_str())
    });
    let dirty = Signal::derive({
        let original = profile.clone();
        move || {
            name.get().trim() != original.name
                || command.get().trim() != agent_command_preview(&original.command_argv)
                || mode.get() != original.mode
        }
    });
    let save_profile = save_agent_profile_handler(&profile.id, state, name, command, mode, saving);
    let delete_profile = delete_agent_profile_handler(&profile.id, state);

    view! {
        <tr class="account-table__row">
            <td>
                <label class="account-form__field account-form__field--compact">
                    <span class="sr-only">"Profile name"</span>
                    <input type="text" prop:value=move || name.get() on:input=move |event| name.set(event_target_value(&event)) />
                </label>
            </td>
            <td>
                <label class="account-form__field account-form__field--compact">
                    <span class="sr-only">"ACP launch command"</span>
                    <textarea rows="2" prop:value=move || command.get() on:input=move |event| command.set(event_target_value(&event)) />
                </label>
            </td>
            <td>
                <label class="account-form__field account-form__field--compact">
                    <span class="sr-only">"Launch mode"</span>
                    <select
                        prop:value=move || profile_mode_value(mode.get())
                        on:change=move |event| mode.set(profile_mode_from_value(&event_target_value(&event)))
                    >
                        <option value="host">"Host"</option>
                        <option value="chroot">"Chroot"</option>
                    </select>
                </label>
            </td>
            <td>
                <div class="account-row__actions-toolbar">
                    <button
                        type="button"
                        class="account-row__action-btn icon-action"
                        prop:disabled=move || saving.get() || !dirty.get()
                        on:click=move |_| save_profile.run(())
                        aria-label=move || agent_profile_save_label(saving.get())
                        title=move || agent_profile_save_label(saving.get())
                    >
                        {move || app_icon_view(if saving.get() { AppIcon::Busy } else { AppIcon::Save })}
                        <span class="sr-only">{move || agent_profile_save_label(saving.get())}</span>
                    </button>
                    <button
                        type="button"
                        class="account-row__action-btn account-row__delete icon-action icon-action--danger"
                        prop:disabled=move || deleting.get()
                        on:click=move |event| delete_profile.run(event)
                        aria-label=move || if deleting.get() { "Deleting…" } else { "Delete profile" }
                        title=move || if deleting.get() { "Deleting…" } else { "Delete profile" }
                    >
                        {move || app_icon_view(if deleting.get() { AppIcon::Busy } else { AppIcon::Delete })}
                        <span class="sr-only">{move || if deleting.get() { "Deleting…" } else { "Delete profile" }}</span>
                    </button>
                </div>
            </td>
        </tr>
    }
    .into_any()
}

#[cfg(target_family = "wasm")]
fn create_agent_profile_submit_handler(
    state: AccountsPageState,
) -> impl Fn(web_sys::SubmitEvent) + Copy + 'static {
    move |event| {
        event.prevent_default();
        if state.agent_profile_saving.get_untracked() {
            return;
        }
        state.agent_profile_saving.set(true);
        state.error.set(None);
        state.notice.set(None);
        let name = state.agent_profile_name.get_untracked();
        let command = state.agent_profile_command.get_untracked();
        let mode = state.agent_profile_mode.get_untracked();
        leptos::task::spawn_local(async move {
            match api::create_agent_profile(name, command, mode).await {
                Ok(profile) => {
                    state.agent_profiles.update(|profiles| profiles.push(profile));
                    state.agent_profile_name.set(String::new());
                    state.agent_profile_command.set(String::new());
                    state.agent_profile_mode.set(AgentProfileMode::Host);
                    state.agent_profile_saving.set(false);
                    state.notice.set(Some("ACP profile saved.".to_string()));
                    spawn_agent_profiles_reload(state);
                }
                Err(message) => {
                    state.agent_profile_saving.set(false);
                    state.error.set(Some(message));
                }
            }
        });
    }
}

#[cfg(not(target_family = "wasm"))]
fn create_agent_profile_submit_handler(
    state: AccountsPageState,
) -> impl Fn(web_sys::SubmitEvent) + Copy + 'static {
    move |_event| {
        if state.agent_profile_saving.get_untracked() {
            return;
        }
        state.agent_profile_saving.set(true);
        state.agent_profile_name.set(String::new());
        state.agent_profile_command.set(String::new());
        state.agent_profile_mode.set(AgentProfileMode::Host);
        state.notice.set(Some("ACP profile saved.".to_string()));
        state.agent_profile_saving.set(false);
    }
}

fn save_agent_profile_handler(
    profile_id: &str,
    state: AccountsPageState,
    name: RwSignal<String>,
    command: RwSignal<String>,
    mode: RwSignal<AgentProfileMode>,
    saving: RwSignal<bool>,
) -> Callback<()> {
    let profile_id = profile_id.to_string();
    Callback::new(move |()| {
        if saving.get_untracked() {
            return;
        }
        saving.set(true);
        state.error.set(None);
        state.notice.set(None);
        save_agent_profile(profile_id.clone(), state, name, command, mode, saving);
    })
}

#[cfg(target_family = "wasm")]
fn save_agent_profile(
    profile_id: String,
    state: AccountsPageState,
    name: RwSignal<String>,
    command: RwSignal<String>,
    mode: RwSignal<AgentProfileMode>,
    saving: RwSignal<bool>,
) {
    leptos::task::spawn_local(async move {
        match api::update_agent_profile(
            &profile_id,
            name.get_untracked(),
            command.get_untracked(),
            mode.get_untracked(),
        )
        .await
        {
            Ok(profile) => {
                state.agent_profiles.update(|profiles| {
                    if let Some(existing) =
                        profiles.iter_mut().find(|existing| existing.id == profile.id)
                    {
                        *existing = profile;
                    }
                });
                saving.set(false);
                state.notice.set(Some("ACP profile updated.".to_string()));
            }
            Err(message) => {
                saving.set(false);
                state.error.set(Some(message));
            }
        }
    });
}

#[cfg(not(target_family = "wasm"))]
fn save_agent_profile(
    _profile_id: String,
    state: AccountsPageState,
    _name: RwSignal<String>,
    _command: RwSignal<String>,
    _mode: RwSignal<AgentProfileMode>,
    saving: RwSignal<bool>,
) {
    saving.set(false);
    state.notice.set(Some("ACP profile updated.".to_string()));
}

fn delete_agent_profile_handler(
    profile_id: &str,
    state: AccountsPageState,
) -> Callback<web_sys::MouseEvent> {
    let profile_id = profile_id.to_string();
    Callback::new(move |_event| {
        if state.deleting_agent_profile_id.get_untracked().is_some() {
            return;
        }
        state.deleting_agent_profile_id.set(Some(profile_id.clone()));
        state.error.set(None);
        state.notice.set(None);
        delete_agent_profile(profile_id.clone(), state);
    })
}

#[cfg(target_family = "wasm")]
fn delete_agent_profile(profile_id: String, state: AccountsPageState) {
    leptos::task::spawn_local(async move {
        match api::delete_agent_profile(&profile_id).await {
            Ok(_) => {
                state
                    .agent_profiles
                    .update(|profiles| profiles.retain(|profile| profile.id != profile_id));
                state.deleting_agent_profile_id.set(None);
                state.notice.set(Some("ACP profile deleted.".to_string()));
            }
            Err(message) => {
                state.deleting_agent_profile_id.set(None);
                state.error.set(Some(message));
            }
        }
    });
}

#[cfg(not(target_family = "wasm"))]
fn delete_agent_profile(profile_id: String, state: AccountsPageState) {
    state
        .agent_profiles
        .update(|profiles| profiles.retain(|profile| profile.id != profile_id));
    state.deleting_agent_profile_id.set(None);
    state.notice.set(Some("ACP profile deleted.".to_string()));
}

fn profile_mode_value(mode: AgentProfileMode) -> &'static str {
    match mode {
        AgentProfileMode::Host => "host",
        AgentProfileMode::Chroot => "chroot",
    }
}

fn profile_mode_from_value(value: &str) -> AgentProfileMode {
    match value {
        "chroot" => AgentProfileMode::Chroot,
        _ => AgentProfileMode::Host,
    }
}

fn agent_profile_save_label(saving: bool) -> &'static str {
    if saving { "Saving…" } else { "Save profile" }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_agent_profile(mode: AgentProfileMode) -> AgentProfile {
        AgentProfile {
            id: "opencode".to_string(),
            name: "OpenCode ACP".to_string(),
            mode,
            command_argv: vec![
                "opencode".to_string(),
                "acp".to_string(),
                "--port".to_string(),
                "${ACP_PORT}".to_string(),
            ],
            env_allowlist: Vec::new(),
            timeout_seconds: 30,
            run_uid: 65_534,
            run_gid: 65_534,
        }
    }

    #[test]
    fn agent_profiles_section_builds_with_host_default() {
        let owner = Owner::new();
        owner.with(|| {
            let state = AccountsPageState::new();
            state
                .agent_profiles
                .set(vec![sample_agent_profile(AgentProfileMode::Host)]);
            let _ = view! { <AgentProfilesSection state=state /> };
            assert_eq!(state.agent_profile_mode.get(), AgentProfileMode::Host);
        });
    }

    #[test]
    fn profile_mode_helpers_match_api_values() {
        assert_eq!(profile_mode_value(AgentProfileMode::Host), "host");
        assert_eq!(profile_mode_value(AgentProfileMode::Chroot), "chroot");
        assert_eq!(profile_mode_from_value("host"), AgentProfileMode::Host);
        assert_eq!(profile_mode_from_value("chroot"), AgentProfileMode::Chroot);
    }

    #[test]
    fn agent_command_preview_preserves_argv_boundaries() {
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
}
