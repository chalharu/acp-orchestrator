use leptos::prelude::*;

use crate::presentation::{AppIcon, app_icon_view};
use crate::routing::SettingsSection;
use crate::{application::auth::AccountsRouteAccess, components::ErrorBanner};

use super::{
    agent_profiles::AgentProfilesSection,
    create_account::CreateAccountSection,
    registry::current_accounts_section,
    shared::{
        AccountsPageState, accounts_back_to_chat_path_from_location, accounts_page_shows_sign_out,
        initialize_accounts_page, sign_out_button_label, sign_out_handler,
        sign_out_redirect_path_from_location,
    },
};

fn accounts_back_to_chat_label() -> &'static str {
    "Back to chat"
}

fn accounts_back_link_view(back_to_chat_href: &str) -> AnyView {
    view! {
        <a
            href=back_to_chat_href.to_string()
            class="account-panel__header-action icon-action icon-action--ghost"
            aria-label=accounts_back_to_chat_label()
            title=accounts_back_to_chat_label()
        >
            {app_icon_view(AppIcon::BackToChat)}
            <span class="sr-only">{accounts_back_to_chat_label()}</span>
        </a>
    }
    .into_any()
}

fn accounts_sign_out_icon(signing_out: bool) -> AppIcon {
    if signing_out {
        AppIcon::Busy
    } else {
        AppIcon::SignOut
    }
}

#[component]
pub fn AccountsPage(section: SettingsSection) -> impl IntoView {
    let state = AccountsPageState::new();
    let back_to_chat_href = accounts_back_to_chat_path_from_location();
    let signing_out = RwSignal::new(false);
    let sign_out = sign_out_handler(
        state.error,
        signing_out,
        sign_out_redirect_path_from_location(),
    );
    initialize_accounts_page(state);

    accounts_page_shell(state, back_to_chat_href, signing_out, sign_out, section)
}

#[cfg(target_family = "wasm")]
fn accounts_page_shell(
    state: AccountsPageState,
    back_to_chat_href: String,
    signing_out: RwSignal<bool>,
    sign_out: Callback<web_sys::MouseEvent>,
    section: SettingsSection,
) -> impl IntoView {
    view! {
        <main class="app-shell account-shell">
            <ErrorBanner message=state.error />
            <section class="panel account-panel">
                <div class="account-panel__header">
                    <h1>"Settings"</h1>
                    <div class="account-panel__header-actions">
                        {accounts_back_link_view(&back_to_chat_href)}
                        <Show when=move || accounts_page_shows_sign_out(state.access.get())>
                            <button
                                type="button"
                                class="account-panel__header-action icon-action icon-action--ghost"
                                on:click=move |event| sign_out.run(event)
                                prop:disabled=move || signing_out.get()
                                aria-label=move || sign_out_button_label(signing_out.get())
                                title=move || sign_out_button_label(signing_out.get())
                            >
                                {move || app_icon_view(accounts_sign_out_icon(signing_out.get()))}
                                <span class="sr-only">{move || sign_out_button_label(signing_out.get())}</span>
                            </button>
                        </Show>
                    </div>
                </div>
                <Show when=move || state.notice.get().is_some()>
                    <p class="account-notice" role="status">
                        {move || state.notice.get().unwrap_or_default()}
                    </p>
                </Show>
                <AccountsPageContent state section />
            </section>
        </main>
    }
}

#[cfg(not(target_family = "wasm"))]
fn accounts_sign_out_button(
    show_sign_out: bool,
    signing_out: bool,
    sign_out: Callback<web_sys::MouseEvent>,
) -> AnyView {
    if show_sign_out {
        let label = sign_out_button_label(signing_out);
        view! {
            <button
                type="button"
                class="account-panel__header-action icon-action icon-action--ghost"
                on:click=move |event| sign_out.run(event)
                prop:disabled=signing_out
                aria-label=label
                title=label
            >
                {app_icon_view(accounts_sign_out_icon(signing_out))}
                <span class="sr-only">{label}</span>
            </button>
        }
        .into_any()
    } else {
        ().into_any()
    }
}

#[cfg(not(target_family = "wasm"))]
fn accounts_notice_view(notice: Option<String>) -> AnyView {
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
fn accounts_page_shell(
    state: AccountsPageState,
    back_to_chat_href: String,
    signing_out: RwSignal<bool>,
    sign_out: Callback<web_sys::MouseEvent>,
    section: SettingsSection,
) -> impl IntoView {
    let show_sign_out = accounts_page_shows_sign_out(state.access.get_untracked());
    let sign_out_button =
        accounts_sign_out_button(show_sign_out, signing_out.get_untracked(), sign_out);
    let notice_view = accounts_notice_view(state.notice.get_untracked());

    view! {
        <main class="app-shell account-shell">
            <ErrorBanner message=state.error />
            <section class="panel account-panel">
                <div class="account-panel__header">
                    <h1>"Settings"</h1>
                    <div class="account-panel__header-actions">
                        {accounts_back_link_view(&back_to_chat_href)}
                        {sign_out_button}
                    </div>
                </div>
                {notice_view}
                <AccountsPageContent state section />
            </section>
        </main>
    }
}

#[component]
fn AccountsPageContent(state: AccountsPageState, section: SettingsSection) -> impl IntoView {
    accounts_page_content(state, section)
}

#[cfg(target_family = "wasm")]
fn accounts_page_content(state: AccountsPageState, section: SettingsSection) -> impl IntoView {
    move || accounts_page_content_body(state.access.get(), state, section)
}

#[cfg(not(target_family = "wasm"))]
fn accounts_page_content(state: AccountsPageState, section: SettingsSection) -> impl IntoView {
    accounts_page_content_body(state.access.get_untracked(), state, section)
}

fn accounts_page_content_body(
    access: Option<AccountsRouteAccess>,
    state: AccountsPageState,
    section: SettingsSection,
) -> AnyView {
    match access {
        Some(AccountsRouteAccess::Admin(_)) => view! {
            <div class="settings-layout">
                {settings_sidebar(section)}
                <div class="settings-content">
                    {settings_section_content(section, state)}
                </div>
            </div>
        }
        .into_any(),
        Some(AccountsRouteAccess::RegisterRequired) => view! {
            <p class="muted">
                "Bootstrap registration is still required. "
                <a href="/app/register/">"Create the first account."</a>
            </p>
        }
        .into_any(),
        Some(AccountsRouteAccess::SignInRequired) => view! {
            <p class="muted">
                "Sign in is required before managing accounts. "
                <a href="/app/sign-in/">"Open sign-in."</a>
            </p>
        }
        .into_any(),
        Some(AccountsRouteAccess::Forbidden) => view! {
            <p class="muted">"This page is available only to admin accounts."</p>
        }
        .into_any(),
        None => view! { <p class="muted">"Checking account access…"</p> }.into_any(),
    }
}

fn settings_sidebar(active: SettingsSection) -> AnyView {
    view! {
        <aside class="settings-sidebar" aria-label="Settings sections">
            <p class="settings-sidebar__eyebrow">"Settings"</p>
            <nav class="settings-sidebar__nav">
                {settings_nav_link(active, SettingsSection::Accounts)}
                {settings_nav_link(active, SettingsSection::Agents)}
            </nav>
        </aside>
    }
    .into_any()
}

fn settings_nav_link(active: SettingsSection, section: SettingsSection) -> AnyView {
    let is_active = active == section;
    view! {
        <a
            class=settings_nav_link_class(is_active)
            href=settings_section_href(section)
            aria-current=settings_nav_aria_current(is_active)
        >
            <span class="settings-nav__title">{settings_section_label(section)}</span>
            <span class="settings-nav__description">{settings_section_description(section)}</span>
        </a>
    }
    .into_any()
}

fn settings_section_content(section: SettingsSection, state: AccountsPageState) -> AnyView {
    match section {
        SettingsSection::Accounts => view! {
            <CreateAccountSection state />
            {current_accounts_section(state)}
        }
        .into_any(),
        SettingsSection::Agents => view! { <AgentProfilesSection state /> }.into_any(),
    }
}

fn settings_nav_link_class(active: bool) -> &'static str {
    if active {
        "settings-nav__link settings-nav__link--active"
    } else {
        "settings-nav__link"
    }
}

fn settings_nav_aria_current(active: bool) -> Option<&'static str> {
    active.then_some("page")
}

fn settings_section_href(section: SettingsSection) -> &'static str {
    match section {
        SettingsSection::Accounts => "/app/settings/accounts/",
        SettingsSection::Agents => "/app/settings/agents/",
    }
}

fn settings_section_label(section: SettingsSection) -> &'static str {
    match section {
        SettingsSection::Accounts => "Accounts",
        SettingsSection::Agents => "Agents",
    }
}

fn settings_section_description(section: SettingsSection) -> &'static str {
    match section {
        SettingsSection::Accounts => "Users and access",
        SettingsSection::Agents => "ACP launch profiles",
    }
}

#[cfg(test)]
mod tests {
    use acp_contracts_accounts::LocalAccount;
    use chrono::{TimeZone, Utc};
    use leptos::prelude::*;

    use super::*;

    fn sample_account(user_id: &str, is_admin: bool) -> LocalAccount {
        LocalAccount {
            user_id: user_id.to_string(),
            username: user_id.to_string(),
            is_admin,
            created_at: Utc.with_ymd_and_hms(2026, 4, 17, 1, 0, 0).unwrap(),
        }
    }

    #[test]
    fn accounts_page_content_builds_for_each_access_state() {
        let owner = Owner::new();
        owner.with(|| {
            let state = AccountsPageState::new();
            let admin = sample_account("admin", true);

            state.access.set(Some(AccountsRouteAccess::Admin(admin)));
            let _ = view! { <AccountsPageContent state=state section=SettingsSection::Accounts /> };
            let _ = view! { <AccountsPageContent state=state section=SettingsSection::Agents /> };

            state
                .access
                .set(Some(AccountsRouteAccess::RegisterRequired));
            let _ = view! { <AccountsPageContent state=state section=SettingsSection::Accounts /> };

            state.access.set(Some(AccountsRouteAccess::SignInRequired));
            let _ = view! { <AccountsPageContent state=state section=SettingsSection::Accounts /> };

            state.access.set(Some(AccountsRouteAccess::Forbidden));
            let _ = view! { <AccountsPageContent state=state section=SettingsSection::Accounts /> };
        });
    }

    #[test]
    fn accounts_page_and_shell_render_host_safe_views() {
        let owner = Owner::new();
        owner.with(|| {
            let state = AccountsPageState::new();
            state.notice.set(Some("Account updated.".to_string()));
            state
                .access
                .set(Some(AccountsRouteAccess::Admin(sample_account(
                    "admin", true,
                ))));
            let _ = accounts_page_shell(
                state,
                "/app/sessions/abc".to_string(),
                RwSignal::new(false),
                Callback::new(|_: web_sys::MouseEvent| {}),
                SettingsSection::Agents,
            );
            let _ = view! { <AccountsPage section=SettingsSection::Accounts /> };
        });
    }

    #[test]
    fn account_header_labels_and_icons_are_stable() {
        assert_eq!(accounts_back_to_chat_label(), "Back to chat");
        assert_eq!(accounts_sign_out_icon(false), AppIcon::SignOut);
        assert_eq!(accounts_sign_out_icon(true), AppIcon::Busy);
    }

    #[test]
    fn account_header_helpers_build_host_safe_views() {
        let owner = Owner::new();
        owner.with(|| {
            let sign_out = Callback::new(|_: web_sys::MouseEvent| {});
            let _ = accounts_back_link_view("/app/sessions/demo");
            let _ = accounts_sign_out_button(true, false, sign_out);
            let _ = accounts_sign_out_button(true, true, sign_out);
            let _ = accounts_sign_out_button(false, false, sign_out);
        });
    }

    #[test]
    fn settings_sidebar_helpers_match_sections() {
        let owner = Owner::new();
        owner.with(|| {
            let _ = settings_sidebar(SettingsSection::Accounts);
            let _ = settings_sidebar(SettingsSection::Agents);
            let _ = settings_nav_link(SettingsSection::Accounts, SettingsSection::Accounts);
            let _ = settings_section_content(SettingsSection::Accounts, AccountsPageState::new());
            let _ = settings_section_content(SettingsSection::Agents, AccountsPageState::new());
        });
        assert_eq!(
            settings_nav_link_class(true),
            "settings-nav__link settings-nav__link--active"
        );
        assert_eq!(settings_nav_link_class(false), "settings-nav__link");
        assert_eq!(settings_nav_aria_current(true), Some("page"));
        assert_eq!(settings_nav_aria_current(false), None);
        assert_eq!(
            settings_section_href(SettingsSection::Accounts),
            "/app/settings/accounts/"
        );
        assert_eq!(
            settings_section_href(SettingsSection::Agents),
            "/app/settings/agents/"
        );
        assert_eq!(
            settings_section_label(SettingsSection::Accounts),
            "Accounts"
        );
        assert_eq!(settings_section_label(SettingsSection::Agents), "Agents");
        assert_eq!(
            settings_section_description(SettingsSection::Accounts),
            "Users and access"
        );
        assert_eq!(
            settings_section_description(SettingsSection::Agents),
            "ACP launch profiles"
        );
    }
}
