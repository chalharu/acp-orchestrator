#![cfg_attr(not(target_family = "wasm"), allow(dead_code))]

use acp_contracts::LocalAccount;
use leptos::prelude::*;
#[cfg(target_family = "wasm")]
use wasm_bindgen::JsCast;

use crate::{
    application::auth::{
        AccountCapabilities, AccountConstraintReason, AccountsRouteAccess, account_capabilities,
    },
    components::ErrorBanner,
    domain::routing::{AppRoute, app_session_path, route_from_pathname},
    infrastructure::api,
};

#[component]
pub fn SessionSidebarAuthControls(
    current_session_id: String,
    error: RwSignal<Option<String>>,
) -> impl IntoView {
    let is_admin = RwSignal::new(false);
    let signed_in = RwSignal::new(false);
    let checked = RwSignal::new(false);
    let signing_out = RwSignal::new(false);
    let accounts_href = accounts_path_with_return_to(&app_session_path(&current_session_id));
    let sign_out = sign_out_handler(error, signing_out);

    initialize_session_sidebar_auth_controls(checked, signed_in, is_admin, error);

    session_sidebar_auth_controls_view(accounts_href, is_admin, signed_in, signing_out, sign_out)
}

#[cfg(target_family = "wasm")]
fn session_sidebar_auth_controls_view(
    accounts_href: String,
    is_admin: RwSignal<bool>,
    signed_in: RwSignal<bool>,
    signing_out: RwSignal<bool>,
    sign_out: Callback<web_sys::MouseEvent>,
) -> impl IntoView {
    view! {
        <Show when=move || is_admin.get()>
            <a class="session-sidebar__secondary-link" href=accounts_href.clone()>
                "Accounts"
            </a>
        </Show>
        <Show when=move || signed_in.get()>
            <button
                type="button"
                class="session-sidebar__secondary-link session-sidebar__secondary-button"
                prop:disabled=move || signing_out.get()
                on:click=move |event| sign_out.run(event)
            >
                {move || sign_out_button_label(signing_out.get())}
            </button>
        </Show>
    }
}

#[cfg(not(target_family = "wasm"))]
fn session_sidebar_auth_controls_view(
    accounts_href: String,
    is_admin: RwSignal<bool>,
    signed_in: RwSignal<bool>,
    signing_out: RwSignal<bool>,
    sign_out: Callback<web_sys::MouseEvent>,
) -> impl IntoView {
    let accounts_link = if is_admin.get_untracked() {
        view! {
            <a class="session-sidebar__secondary-link" href=accounts_href>
                "Accounts"
            </a>
        }
        .into_any()
    } else {
        ().into_any()
    };
    let sign_out_button = if signed_in.get_untracked() {
        let signing_out = signing_out.get_untracked();
        let label = sign_out_button_label(signing_out);
        view! {
            <button
                type="button"
                class="session-sidebar__secondary-link session-sidebar__secondary-button"
                prop:disabled=signing_out
                on:click=move |event| sign_out.run(event)
            >
                {label}
            </button>
        }
        .into_any()
    } else {
        ().into_any()
    };

    view! {
        {accounts_link}
        {sign_out_button}
    }
}

#[cfg(target_family = "wasm")]
fn initialize_session_sidebar_auth_controls(
    checked: RwSignal<bool>,
    signed_in: RwSignal<bool>,
    is_admin: RwSignal<bool>,
    error: RwSignal<Option<String>>,
) {
    Effect::new(move |_| {
        if checked.get() {
            return;
        }
        checked.set(true);
        leptos::task::spawn_local(async move {
            match api::auth_status().await {
                Ok(status) => {
                    signed_in.set(status.account.is_some());
                    is_admin.set(status.account.is_some_and(|account| account.is_admin));
                }
                Err(message) => error.set(Some(message)),
            }
        });
    });
}

#[cfg(not(target_family = "wasm"))]
fn initialize_session_sidebar_auth_controls(
    checked: RwSignal<bool>,
    _signed_in: RwSignal<bool>,
    _is_admin: RwSignal<bool>,
    _error: RwSignal<Option<String>>,
) {
    initialize_session_sidebar_auth_controls_host(checked);
}

#[cfg(not(target_family = "wasm"))]
fn initialize_session_sidebar_auth_controls_host(checked: RwSignal<bool>) {
    if checked.get_untracked() {
        return;
    }

    checked.set(true);
}

#[derive(Clone, Copy)]
struct AccountsPageState {
    error: RwSignal<Option<String>>,
    notice: RwSignal<Option<String>>,
    access: RwSignal<Option<AccountsRouteAccess>>,
    current_user_id: RwSignal<String>,
    accounts: RwSignal<Vec<LocalAccount>>,
    loading_accounts: RwSignal<bool>,
    create_username: RwSignal<String>,
    create_password: RwSignal<String>,
    create_admin: RwSignal<bool>,
    creating: RwSignal<bool>,
    checked: RwSignal<bool>,
}

impl AccountsPageState {
    fn new() -> Self {
        Self {
            error: RwSignal::new(None::<String>),
            notice: RwSignal::new(None::<String>),
            access: RwSignal::new(None::<AccountsRouteAccess>),
            current_user_id: RwSignal::new(String::new()),
            accounts: RwSignal::new(Vec::<LocalAccount>::new()),
            loading_accounts: RwSignal::new(true),
            create_username: RwSignal::new(String::new()),
            create_password: RwSignal::new(String::new()),
            create_admin: RwSignal::new(false),
            creating: RwSignal::new(false),
            checked: RwSignal::new(false),
        }
    }
}

#[component]
pub fn AccountsPage() -> impl IntoView {
    let state = AccountsPageState::new();
    let back_to_chat_href = accounts_back_to_chat_path_from_location();
    let signing_out = RwSignal::new(false);
    let sign_out = sign_out_handler(state.error, signing_out);
    initialize_accounts_page(state);

    accounts_page_shell(state, back_to_chat_href, signing_out, sign_out)
}

#[cfg(target_family = "wasm")]
fn accounts_page_shell(
    state: AccountsPageState,
    back_to_chat_href: String,
    signing_out: RwSignal<bool>,
    sign_out: Callback<web_sys::MouseEvent>,
) -> impl IntoView {
    view! {
        <main class="app-shell account-shell">
            <ErrorBanner message=state.error />
            <section class="panel account-panel">
                <div class="account-panel__header">
                    <h1>"Accounts"</h1>
                    <div class="account-panel__header-actions">
                        <a href=back_to_chat_href>"Back to chat"</a>
                        <Show when=move || accounts_page_shows_sign_out(state.access.get())>
                            <button
                                type="button"
                                on:click=move |event| sign_out.run(event)
                                prop:disabled=move || signing_out.get()
                            >
                                {move || sign_out_button_label(signing_out.get())}
                            </button>
                        </Show>
                    </div>
                </div>
                <Show when=move || state.notice.get().is_some()>
                    <p class="account-notice" role="status">
                        {move || state.notice.get().unwrap_or_default()}
                    </p>
                </Show>
                <AccountsPageContent state />
            </section>
        </main>
    }
}

#[cfg(not(target_family = "wasm"))]
fn accounts_page_shell(
    state: AccountsPageState,
    back_to_chat_href: String,
    signing_out: RwSignal<bool>,
    sign_out: Callback<web_sys::MouseEvent>,
) -> impl IntoView {
    let show_sign_out = accounts_page_shows_sign_out(state.access.get_untracked());
    let sign_out_button = if show_sign_out {
        let signing_out = signing_out.get_untracked();
        let label = sign_out_button_label(signing_out);
        view! {
            <button
                type="button"
                on:click=move |event| sign_out.run(event)
                prop:disabled=signing_out
            >
                {label}
            </button>
        }
        .into_any()
    } else {
        ().into_any()
    };
    let notice = state.notice.get_untracked();
    let notice_view = if let Some(notice) = notice {
        view! {
            <p class="account-notice" role="status">
                {notice}
            </p>
        }
        .into_any()
    } else {
        ().into_any()
    };

    view! {
        <main class="app-shell account-shell">
            <ErrorBanner message=state.error />
            <section class="panel account-panel">
                <div class="account-panel__header">
                    <h1>"Accounts"</h1>
                    <div class="account-panel__header-actions">
                        <a href=back_to_chat_href>"Back to chat"</a>
                        {sign_out_button}
                    </div>
                </div>
                {notice_view}
                <AccountsPageContent state />
            </section>
        </main>
    }
}

#[cfg(target_family = "wasm")]
fn initialize_accounts_page(state: AccountsPageState) {
    Effect::new(move |_| {
        if state.checked.get() {
            return;
        }

        state.checked.set(true);
        leptos::task::spawn_local(async move {
            match api::auth_status().await {
                Ok(status) => {
                    let access = crate::application::auth::accounts_route_access(&status);
                    let should_load_accounts = matches!(access, AccountsRouteAccess::Admin(_));
                    state.access.set(Some(access));
                    if should_load_accounts {
                        spawn_account_reload(state);
                    } else {
                        state.loading_accounts.set(false);
                    }
                }
                Err(message) => {
                    state.loading_accounts.set(false);
                    state.error.set(Some(message));
                }
            }
        });
    });
}

#[cfg(not(target_family = "wasm"))]
fn initialize_accounts_page(state: AccountsPageState) {
    initialize_accounts_page_host(state);
}

#[cfg(not(target_family = "wasm"))]
fn initialize_accounts_page_host(state: AccountsPageState) {
    if state.checked.get_untracked() {
        return;
    }

    state.checked.set(true);
    state.loading_accounts.set(false);
}

#[component]
#[cfg(target_family = "wasm")]
fn AccountsPageContent(state: AccountsPageState) -> impl IntoView {
    move || match state.access.get() {
        Some(AccountsRouteAccess::Admin(_)) => view! {
            <CreateAccountSection state />
            <CurrentAccountsSection state />
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

#[component]
#[cfg(not(target_family = "wasm"))]
fn AccountsPageContent(state: AccountsPageState) -> impl IntoView {
    match state.access.get_untracked() {
        Some(AccountsRouteAccess::Admin(_)) => view! {
            <CreateAccountSection state />
            <CurrentAccountsSection state />
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

#[component]
#[cfg(target_family = "wasm")]
fn CreateAccountSection(state: AccountsPageState) -> impl IntoView {
    let on_submit = create_account_submit_handler(state);

    view! {
        <div class="account-panel__section">
            <div class="account-panel__section-heading">
                <div class="account-panel__section-copy">
                    <h2>"Create account"</h2>
                    <p class="muted">
                        "Provision a browser sign-in with an optional admin grant."
                    </p>
                </div>
            </div>
            <form class="account-form account-form--create" on:submit=on_submit>
                <label class="account-form__field">
                    <span>"User name"</span>
                    <input
                        type="text"
                        prop:value=move || state.create_username.get()
                        on:input=move |event| state.create_username.set(event_target_value(&event))
                    />
                </label>
                <label class="account-form__field">
                    <span>"Password"</span>
                    <input
                        type="password"
                        prop:value=move || state.create_password.get()
                        on:input=move |event| state.create_password.set(event_target_value(&event))
                    />
                </label>
                <label class="account-checkbox">
                    <input
                        type="checkbox"
                        prop:checked=move || state.create_admin.get()
                        on:change=move |event| state.create_admin.set(event_target_checked(&event))
                    />
                    <span>"Admin"</span>
                </label>
                <button
                    type="submit"
                    class="account-form__submit"
                    prop:disabled=move || state.creating.get()
                >
                    {move || if state.creating.get() { "Saving…" } else { "Create account" }}
                </button>
            </form>
        </div>
    }
}

#[component]
#[cfg(not(target_family = "wasm"))]
fn CreateAccountSection(state: AccountsPageState) -> impl IntoView {
    let on_submit = create_account_submit_handler(state);
    let creating = state.creating.get_untracked();
    let create_username = state.create_username.get_untracked();
    let create_password = state.create_password.get_untracked();
    let create_admin = state.create_admin.get_untracked();

    view! {
        <div class="account-panel__section">
            <div class="account-panel__section-heading">
                <div class="account-panel__section-copy">
                    <h2>"Create account"</h2>
                    <p class="muted">
                        "Provision a browser sign-in with an optional admin grant."
                    </p>
                </div>
            </div>
            <form class="account-form account-form--create" on:submit=on_submit>
                <label class="account-form__field">
                    <span>"User name"</span>
                    <input
                        type="text"
                        prop:value=create_username
                        on:input=move |event| state.create_username.set(event_target_value(&event))
                    />
                </label>
                <label class="account-form__field">
                    <span>"Password"</span>
                    <input
                        type="password"
                        prop:value=create_password
                        on:input=move |event| state.create_password.set(event_target_value(&event))
                    />
                </label>
                <label class="account-checkbox">
                    <input
                        type="checkbox"
                        prop:checked=create_admin
                        on:change=move |event| state.create_admin.set(event_target_checked(&event))
                    />
                    <span>"Admin"</span>
                </label>
                <button type="submit" class="account-form__submit" prop:disabled=creating>
                    {create_account_button_label(creating)}
                </button>
            </form>
        </div>
    }
}

#[cfg(target_family = "wasm")]
fn create_account_submit_handler(
    state: AccountsPageState,
) -> impl Fn(web_sys::SubmitEvent) + Copy + 'static {
    move |event: web_sys::SubmitEvent| {
        event.prevent_default();
        if state.creating.get_untracked() {
            return;
        }

        state.creating.set(true);
        state.error.set(None);
        state.notice.set(None);
        let username = state.create_username.get_untracked();
        let password = state.create_password.get_untracked();
        let is_admin = state.create_admin.get_untracked();
        leptos::task::spawn_local(async move {
            match api::create_account(&username, &password, is_admin).await {
                Ok(_) => {
                    state.create_username.set(String::new());
                    state.create_password.set(String::new());
                    state.create_admin.set(false);
                    state.notice.set(Some("Account created.".to_string()));
                    state.creating.set(false);
                    spawn_account_reload(state);
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
fn create_account_submit_handler(
    state: AccountsPageState,
) -> impl Fn(web_sys::SubmitEvent) + Copy + 'static {
    move |_event: web_sys::SubmitEvent| create_account_submit_host(state)
}

#[component]
#[cfg(target_family = "wasm")]
fn CurrentAccountsSection(state: AccountsPageState) -> impl IntoView {
    let account_count = Signal::derive(move || state.accounts.get().len());

    view! {
        <div class="account-panel__section account-panel__section--registry">
            <div class="account-panel__section-heading">
                <div class="account-panel__section-copy">
                    <h2>"Account registry"</h2>
                    <p class="muted">
                        "User names stay fixed. Adjust passwords and admin access row by row."
                    </p>
                </div>
                <p class="account-panel__summary">
                    {move || account_count_label(account_count.get())}
                </p>
            </div>
            <Show
                when=move || !state.loading_accounts.get()
                fallback=|| view! { <p class="muted">"Loading accounts…"</p> }
            >
                <div class="account-table-wrap">
                    <table class="account-table">
                        <caption class="sr-only">"Local accounts and admin controls"</caption>
                        <thead>
                            <tr>
                                <th scope="col">"Account"</th>
                                <th scope="col">"State"</th>
                                <th scope="col">"Created"</th>
                                <th scope="col">"Password reset"</th>
                                <th scope="col">"Admin access"</th>
                                <th scope="col">"Actions"</th>
                            </tr>
                        </thead>
                        <tbody>
                            <For
                                each=move || state.accounts.get()
                                key=|account| account.user_id.clone()
                                children=move |account| view! { <AccountRow account state /> }
                            />
                        </tbody>
                    </table>
                </div>
            </Show>
        </div>
    }
}

#[component]
#[cfg(not(target_family = "wasm"))]
fn CurrentAccountsSection(state: AccountsPageState) -> impl IntoView {
    let loading_accounts = state.loading_accounts.get_untracked();
    let summary = account_count_label(state.accounts.get_untracked().len());
    let content = if loading_accounts {
        view! { <p class="muted">"Loading accounts…"</p> }.into_any()
    } else {
        let rows = state
            .accounts
            .get_untracked()
            .into_iter()
            .map(|account| view! { <AccountRow account state /> })
            .collect_view()
            .into_any();
        view! {
            <div class="account-table-wrap">
                <table class="account-table">
                    <caption class="sr-only">"Local accounts and admin controls"</caption>
                    <thead>
                        <tr>
                            <th scope="col">"Account"</th>
                            <th scope="col">"State"</th>
                            <th scope="col">"Created"</th>
                            <th scope="col">"Password reset"</th>
                            <th scope="col">"Admin access"</th>
                            <th scope="col">"Actions"</th>
                        </tr>
                    </thead>
                    <tbody>{rows}</tbody>
                </table>
            </div>
        }
        .into_any()
    };

    view! {
        <div class="account-panel__section account-panel__section--registry">
            <div class="account-panel__section-heading">
                <div class="account-panel__section-copy">
                    <h2>"Account registry"</h2>
                    <p class="muted">
                        "User names stay fixed. Adjust passwords and admin access row by row."
                    </p>
                </div>
                <p class="account-panel__summary">{summary}</p>
            </div>
            {content}
        </div>
    }
}

#[component]
fn AccountRow(account: LocalAccount, state: AccountsPageState) -> impl IntoView {
    let password = RwSignal::new(String::new());
    let admin_checked = RwSignal::new(account.is_admin);
    let saving = RwSignal::new(false);
    let deleting = RwSignal::new(false);
    let row_dirty = account_row_dirty_signal(&account, password, admin_checked);
    let capabilities = account_capabilities_signal(&account, state);
    let role_kind = account_role_kind_signal(&account, state);
    let role_label = account_role_label_signal(role_kind);
    let role_badge_class = account_role_badge_class_signal(role_kind);
    let constraint_label = account_constraint_label_signal(capabilities);
    let can_modify = account_can_modify_signal(capabilities);
    let (save_account, delete_account) =
        account_row_action_handlers(&account, state, password, admin_checked, saving, deleting);
    let created_label = account_created_label(&account);
    let username = account.username.clone();

    view! {
        <tr class="account-table__row">
            <td>
                <AccountRowSummary username=username.clone() constraint_label />
            </td>
            <td>
                <AccountStateCell role_label role_badge_class />
            </td>
            <td class="account-table__created">
                <span>{created_label}</span>
            </td>
            <td>
                <AccountPasswordField password username />
            </td>
            <td>
                <AccountAdminToggle admin_checked can_modify />
            </td>
            <td>
                <AccountRowActions
                    saving
                    deleting
                    row_dirty
                    can_modify
                    save_account
                    delete_account
                    constraint_label
                />
            </td>
        </tr>
    }
}

fn account_row_dirty_signal(
    account: &LocalAccount,
    password: RwSignal<String>,
    admin_checked: RwSignal<bool>,
) -> Signal<bool> {
    let row_dirty_account = account.clone();
    Signal::derive(move || {
        !password.get().trim().is_empty() || admin_checked.get() != row_dirty_account.is_admin
    })
}

fn account_capabilities_signal(
    account: &LocalAccount,
    state: AccountsPageState,
) -> Signal<AccountCapabilities> {
    let capabilities_account = account.clone();
    Signal::derive(move || {
        account_capabilities(
            &state.current_user_id.get(),
            &state.accounts.get(),
            &capabilities_account,
        )
    })
}

fn account_role_kind_signal(
    account: &LocalAccount,
    state: AccountsPageState,
) -> Signal<AccountRoleKind> {
    let role_account = account.clone();
    Signal::derive(move || account_role_kind(&role_account, &state.current_user_id.get()))
}

fn account_role_label_signal(role_kind: Signal<AccountRoleKind>) -> Signal<String> {
    Signal::derive(move || account_role_label(role_kind.get()).to_string())
}

fn account_role_badge_class_signal(role_kind: Signal<AccountRoleKind>) -> Signal<String> {
    Signal::derive(move || {
        format!(
            "account-role-pill account-role-pill--{}",
            account_role_badge_modifier(role_kind.get())
        )
    })
}

fn account_constraint_label_signal(capabilities: Signal<AccountCapabilities>) -> Signal<String> {
    Signal::derive(move || account_constraint_label(capabilities.get().constraint).to_string())
}

fn account_can_modify_signal(capabilities: Signal<AccountCapabilities>) -> Signal<bool> {
    Signal::derive(move || capabilities.get().can_modify())
}

fn account_row_action_handlers(
    account: &LocalAccount,
    state: AccountsPageState,
    password: RwSignal<String>,
    admin_checked: RwSignal<bool>,
    saving: RwSignal<bool>,
    deleting: RwSignal<bool>,
) -> (Callback<()>, Callback<web_sys::MouseEvent>) {
    let account_id = account.user_id.clone();
    (
        account_save_handler(account_id.clone(), state, password, admin_checked, saving),
        account_delete_handler(account_id, state, deleting),
    )
}

#[component]
fn AccountRowSummary(username: String, constraint_label: Signal<String>) -> impl IntoView {
    view! {
        <div class="account-row__summary">
            <strong class="account-row__name">{username}</strong>
            <Show when=move || !constraint_label.get().is_empty()>
                <span class="account-row__note">{move || constraint_label.get()}</span>
            </Show>
        </div>
    }
}

#[component]
fn AccountStateCell(role_label: Signal<String>, role_badge_class: Signal<String>) -> impl IntoView {
    view! {
        <div class="account-state-cell">
            <span class=move || role_badge_class.get()>{move || role_label.get()}</span>
        </div>
    }
}

#[component]
fn AccountPasswordField(password: RwSignal<String>, username: String) -> impl IntoView {
    let input_label = format!("New password for {username}");
    let sr_label = input_label.clone();

    view! {
        <label class="account-form__field account-form__field--compact">
            <span class="sr-only">{sr_label}</span>
            <input
                type="password"
                placeholder="Leave blank to keep current"
                aria-label=input_label
                prop:value=move || password.get()
                on:input=move |event| password.set(event_target_value(&event))
            />
        </label>
    }
}

#[component]
fn AccountAdminToggle(admin_checked: RwSignal<bool>, can_modify: Signal<bool>) -> impl IntoView {
    view! {
        <label class="account-checkbox account-checkbox--table">
            <input
                type="checkbox"
                prop:checked=move || admin_checked.get()
                prop:disabled=move || !can_modify.get()
                on:change=move |event| admin_checked.set(event_target_checked(&event))
            />
            <span>{move || admin_access_label(admin_checked.get())}</span>
        </label>
    }
}

#[component]
#[cfg(target_family = "wasm")]
fn AccountRowActions(
    saving: RwSignal<bool>,
    deleting: RwSignal<bool>,
    row_dirty: Signal<bool>,
    can_modify: Signal<bool>,
    save_account: Callback<()>,
    delete_account: Callback<web_sys::MouseEvent>,
    constraint_label: Signal<String>,
) -> impl IntoView {
    let hint_text =
        account_row_hint_signal(saving, deleting, row_dirty, can_modify, constraint_label);

    view! {
        <div class="account-row__actions">
            <button
                type="button"
                prop:disabled=move || saving.get() || !row_dirty.get()
                on:click=move |_| save_account.run(())
            >
                {move || save_button_label(saving.get())}
            </button>
            <button
                type="button"
                class="account-row__delete"
                prop:disabled=move || deleting.get() || !can_modify.get()
                on:click=move |event| delete_account.run(event)
            >
                {move || delete_button_label(deleting.get())}
            </button>
            <p class="account-row__hint">{move || hint_text.get()}</p>
        </div>
    }
}

#[component]
#[cfg(not(target_family = "wasm"))]
fn AccountRowActions(
    saving: RwSignal<bool>,
    deleting: RwSignal<bool>,
    row_dirty: Signal<bool>,
    can_modify: Signal<bool>,
    save_account: Callback<()>,
    delete_account: Callback<web_sys::MouseEvent>,
    constraint_label: Signal<String>,
) -> impl IntoView {
    let saving_now = saving.get_untracked();
    let deleting_now = deleting.get_untracked();
    let row_dirty_now = row_dirty.get_untracked();
    let can_modify_now = can_modify.get_untracked();
    let hint_text = account_row_hint(
        saving_now,
        deleting_now,
        row_dirty_now,
        can_modify_now,
        constraint_label.get_untracked(),
    );

    view! {
        <div class="account-row__actions">
            <button
                type="button"
                prop:disabled=saving_now || !row_dirty_now
                on:click=move |_| save_account.run(())
            >
                {save_button_label(saving_now)}
            </button>
            <button
                type="button"
                class="account-row__delete"
                prop:disabled=deleting_now || !can_modify_now
                on:click=move |event| delete_account.run(event)
            >
                {delete_button_label(deleting_now)}
            </button>
            <p class="account-row__hint">{hint_text}</p>
        </div>
    }
}

fn account_row_hint_signal(
    saving: RwSignal<bool>,
    deleting: RwSignal<bool>,
    row_dirty: Signal<bool>,
    can_modify: Signal<bool>,
    constraint_label: Signal<String>,
) -> Signal<String> {
    Signal::derive(move || {
        account_row_hint(
            saving.get(),
            deleting.get(),
            row_dirty.get(),
            can_modify.get(),
            constraint_label.get(),
        )
    })
}

fn account_row_hint(
    saving: bool,
    deleting: bool,
    row_dirty: bool,
    can_modify: bool,
    constraint_label: String,
) -> String {
    if saving {
        "Saving changes…".to_string()
    } else if deleting {
        "Removing account…".to_string()
    } else if !row_dirty {
        "No pending changes".to_string()
    } else if !can_modify && !constraint_label.is_empty() {
        constraint_label
    } else {
        "Ready to apply".to_string()
    }
}

#[cfg(target_family = "wasm")]
fn account_save_handler(
    account_user_id: String,
    state: AccountsPageState,
    password: RwSignal<String>,
    admin_checked: RwSignal<bool>,
    saving: RwSignal<bool>,
) -> Callback<()> {
    Callback::new(move |_| {
        if saving.get_untracked() {
            return;
        }

        saving.set(true);
        state.error.set(None);
        state.notice.set(None);
        let password_update = password_update(password.get_untracked());
        let is_admin = Some(admin_checked.get_untracked());
        let account_user_id = account_user_id.clone();
        leptos::task::spawn_local(async move {
            match api::update_account(&account_user_id, password_update, is_admin).await {
                Ok(_) => {
                    password.set(String::new());
                    saving.set(false);
                    state.notice.set(Some("Account updated.".to_string()));
                    spawn_account_reload(state);
                }
                Err(message) => {
                    saving.set(false);
                    state.error.set(Some(message));
                }
            }
        });
    })
}

#[cfg(not(target_family = "wasm"))]
fn account_save_handler(
    account_user_id: String,
    state: AccountsPageState,
    password: RwSignal<String>,
    admin_checked: RwSignal<bool>,
    saving: RwSignal<bool>,
) -> Callback<()> {
    Callback::new(move |_| {
        account_save_host(&account_user_id, state, password, admin_checked, saving)
    })
}

#[cfg(target_family = "wasm")]
fn account_delete_handler(
    account_user_id: String,
    state: AccountsPageState,
    deleting: RwSignal<bool>,
) -> Callback<web_sys::MouseEvent> {
    Callback::new(move |_| {
        if deleting.get_untracked() {
            return;
        }

        deleting.set(true);
        state.error.set(None);
        state.notice.set(None);
        let account_user_id = account_user_id.clone();
        leptos::task::spawn_local(async move {
            match api::delete_account(&account_user_id).await {
                Ok(_) => {
                    deleting.set(false);
                    state.notice.set(Some("Account deleted.".to_string()));
                    spawn_account_reload(state);
                }
                Err(message) => {
                    deleting.set(false);
                    state.error.set(Some(message));
                }
            }
        });
    })
}

#[cfg(not(target_family = "wasm"))]
fn account_delete_handler(
    account_user_id: String,
    state: AccountsPageState,
    deleting: RwSignal<bool>,
) -> Callback<web_sys::MouseEvent> {
    Callback::new(move |_| account_delete_host(&account_user_id, state, deleting))
}

fn password_update(password: String) -> Option<String> {
    if password.trim().is_empty() {
        None
    } else {
        Some(password)
    }
}

fn save_button_label(saving: bool) -> &'static str {
    if saving { "Saving…" } else { "Save" }
}

fn delete_button_label(deleting: bool) -> &'static str {
    if deleting { "Deleting…" } else { "Delete" }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AccountRoleKind {
    Active,
    Admin,
    Member,
}

fn account_role_kind(account: &LocalAccount, current_user_id: &str) -> AccountRoleKind {
    if account.user_id == current_user_id {
        AccountRoleKind::Active
    } else if account.is_admin {
        AccountRoleKind::Admin
    } else {
        AccountRoleKind::Member
    }
}

fn account_role_label(role_kind: AccountRoleKind) -> &'static str {
    match role_kind {
        AccountRoleKind::Active => "signed in",
        AccountRoleKind::Admin => "admin",
        AccountRoleKind::Member => "member",
    }
}

fn account_role_badge_modifier(role_kind: AccountRoleKind) -> &'static str {
    match role_kind {
        AccountRoleKind::Active => "active",
        AccountRoleKind::Admin => "admin",
        AccountRoleKind::Member => "member",
    }
}

fn account_constraint_label(reason: Option<AccountConstraintReason>) -> &'static str {
    match reason {
        Some(AccountConstraintReason::CurrentUser) => "Signed in on this browser",
        Some(AccountConstraintReason::LastAdmin) => "One admin account must remain",
        None => "",
    }
}

fn account_created_label(account: &LocalAccount) -> String {
    account.created_at.format("%Y-%m-%d %H:%M UTC").to_string()
}

fn account_count_label(count: usize) -> String {
    if count == 1 {
        "1 account".to_string()
    } else {
        format!("{count} accounts")
    }
}

fn accounts_path_with_return_to(return_to_path: &str) -> String {
    format!(
        "/app/accounts/?return_to={}",
        api::encode_component(return_to_path)
    )
}

#[cfg(target_family = "wasm")]
fn accounts_back_to_chat_path_from_location() -> String {
    web_sys::window()
        .and_then(|window| window.location().search().ok())
        .map(|search| accounts_back_to_chat_path(&search))
        .unwrap_or_else(|| "/app/".to_string())
}

#[cfg(not(target_family = "wasm"))]
fn accounts_back_to_chat_path_from_location() -> String {
    "/app/".to_string()
}

fn accounts_back_to_chat_path(search: &str) -> String {
    query_param(search, "return_to")
        .filter(|path| matches!(route_from_pathname(path), AppRoute::Session(_)))
        .unwrap_or_else(|| "/app/".to_string())
}

fn query_param(search: &str, name: &str) -> Option<String> {
    search
        .trim_start_matches('?')
        .split('&')
        .filter(|pair| !pair.is_empty())
        .find_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            (key == name)
                .then(|| api::decode_component(value))
                .flatten()
        })
}

fn accounts_page_shows_sign_out(access: Option<AccountsRouteAccess>) -> bool {
    matches!(
        access,
        Some(AccountsRouteAccess::Admin(_)) | Some(AccountsRouteAccess::Forbidden)
    )
}

#[cfg(target_family = "wasm")]
fn sign_out_handler(
    error: RwSignal<Option<String>>,
    signing_out: RwSignal<bool>,
) -> Callback<web_sys::MouseEvent> {
    Callback::new(move |_event: web_sys::MouseEvent| {
        if signing_out.get_untracked() {
            return;
        }

        signing_out.set(true);
        error.set(None);
        leptos::task::spawn_local(async move {
            match api::sign_out().await {
                Ok(()) => {
                    crate::browser::clear_prepared_session_id();
                    if let Err(message) = crate::browser::navigate_to("/app/sign-in/") {
                        signing_out.set(false);
                        error.set(Some(message));
                    }
                }
                Err(message) => {
                    signing_out.set(false);
                    error.set(Some(message));
                }
            }
        });
    })
}

#[cfg(not(target_family = "wasm"))]
fn sign_out_handler(
    error: RwSignal<Option<String>>,
    signing_out: RwSignal<bool>,
) -> Callback<web_sys::MouseEvent> {
    Callback::new(move |_event: web_sys::MouseEvent| sign_out_host(error, signing_out))
}

fn sign_out_button_label(signing_out: bool) -> &'static str {
    if signing_out {
        "Signing out…"
    } else {
        "Sign out"
    }
}

fn admin_access_label(is_admin: bool) -> &'static str {
    if is_admin { "Enabled" } else { "Standard" }
}

#[cfg(target_family = "wasm")]
fn event_target_checked(event: &web_sys::Event) -> bool {
    event
        .target()
        .and_then(|target| target.dyn_into::<web_sys::HtmlInputElement>().ok())
        .map(|input| input.checked())
        .unwrap_or(false)
}

#[cfg(not(target_family = "wasm"))]
fn event_target_checked<T>(_event: &T) -> bool {
    false
}

fn create_account_button_label(creating: bool) -> &'static str {
    if creating {
        "Saving…"
    } else {
        "Create account"
    }
}

#[cfg(not(target_family = "wasm"))]
fn create_account_submit_host(state: AccountsPageState) {
    if state.creating.get_untracked() {
        return;
    }

    state.creating.set(true);
    state.error.set(None);
    state.notice.set(None);
    let _username = state.create_username.get_untracked();
    let _password = state.create_password.get_untracked();
    let _is_admin = state.create_admin.get_untracked();
    state.create_username.set(String::new());
    state.create_password.set(String::new());
    state.create_admin.set(false);
    state.notice.set(Some("Account created.".to_string()));
    state.creating.set(false);
    spawn_account_reload(state);
}

#[cfg(not(target_family = "wasm"))]
fn account_save_host(
    account_user_id: &str,
    state: AccountsPageState,
    password: RwSignal<String>,
    admin_checked: RwSignal<bool>,
    saving: RwSignal<bool>,
) {
    if saving.get_untracked() {
        return;
    }

    saving.set(true);
    state.error.set(None);
    state.notice.set(None);
    let _password_update = password_update(password.get_untracked());
    let _is_admin = Some(admin_checked.get_untracked());
    let _account_user_id = account_user_id.to_string();
    password.set(String::new());
    saving.set(false);
    state.notice.set(Some("Account updated.".to_string()));
    spawn_account_reload(state);
}

#[cfg(not(target_family = "wasm"))]
fn account_delete_host(account_user_id: &str, state: AccountsPageState, deleting: RwSignal<bool>) {
    if deleting.get_untracked() {
        return;
    }

    deleting.set(true);
    state.error.set(None);
    state.notice.set(None);
    let _account_user_id = account_user_id.to_string();
    deleting.set(false);
    state.notice.set(Some("Account deleted.".to_string()));
    spawn_account_reload(state);
}

#[cfg(not(target_family = "wasm"))]
fn sign_out_host(error: RwSignal<Option<String>>, signing_out: RwSignal<bool>) {
    if signing_out.get_untracked() {
        return;
    }

    signing_out.set(true);
    error.set(None);
}

#[cfg(target_family = "wasm")]
fn spawn_account_reload(state: AccountsPageState) {
    state.loading_accounts.set(true);
    state.error.set(None);
    leptos::task::spawn_local(async move {
        match api::list_accounts().await {
            Ok(response) => {
                state.current_user_id.set(response.current_user_id);
                state.accounts.set(response.accounts);
                state.loading_accounts.set(false);
            }
            Err(message) => {
                state.loading_accounts.set(false);
                state.error.set(Some(message));
            }
        }
    });
}

#[cfg(not(target_family = "wasm"))]
fn spawn_account_reload(state: AccountsPageState) {
    state.loading_accounts.set(true);
    state.error.set(None);
    state.loading_accounts.set(false);
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use leptos::prelude::*;
    use wasm_bindgen::{JsCast, JsValue};

    use super::*;

    fn sample_account(user_id: &str, is_admin: bool) -> LocalAccount {
        LocalAccount {
            user_id: user_id.to_string(),
            username: user_id.to_string(),
            is_admin,
            created_at: Utc.with_ymd_and_hms(2026, 4, 17, 1, 0, 0).unwrap(),
        }
    }

    #[cfg(not(target_family = "wasm"))]
    fn fake_submit_event() -> web_sys::SubmitEvent {
        JsValue::NULL.unchecked_into()
    }

    #[cfg(not(target_family = "wasm"))]
    fn fake_mouse_event() -> web_sys::MouseEvent {
        JsValue::NULL.unchecked_into()
    }

    #[test]
    fn password_update_ignores_blank_passwords() {
        assert_eq!(password_update("   ".to_string()), None);
        assert_eq!(
            password_update("password123".to_string()),
            Some("password123".to_string())
        );
    }

    #[test]
    fn account_role_kind_prefers_signed_in_over_admin() {
        let admin = sample_account("admin", true);
        let member = sample_account("member", false);

        assert_eq!(account_role_kind(&admin, "admin"), AccountRoleKind::Active);
        assert_eq!(account_role_kind(&admin, "other"), AccountRoleKind::Admin);
        assert_eq!(account_role_kind(&member, "other"), AccountRoleKind::Member);
        assert_eq!(account_role_label(AccountRoleKind::Active), "signed in");
        assert_eq!(account_role_label(AccountRoleKind::Member), "member");
        assert_eq!(account_role_badge_modifier(AccountRoleKind::Admin), "admin");
    }

    #[test]
    fn account_constraint_label_explains_protected_rows() {
        assert_eq!(
            account_constraint_label(Some(AccountConstraintReason::CurrentUser)),
            "Signed in on this browser"
        );
        assert_eq!(
            account_constraint_label(Some(AccountConstraintReason::LastAdmin)),
            "One admin account must remain"
        );
        assert_eq!(account_constraint_label(None), "");
    }

    #[test]
    fn account_created_label_uses_utc_stamp() {
        assert_eq!(
            account_created_label(&sample_account("member", false)),
            "2026-04-17 01:00 UTC"
        );
    }

    #[test]
    fn account_row_hint_explains_row_state() {
        assert_eq!(
            account_row_hint(false, false, false, true, String::new()),
            "No pending changes"
        );
        assert_eq!(
            account_row_hint(
                false,
                false,
                true,
                false,
                "One admin account must remain".to_string()
            ),
            "One admin account must remain"
        );
        assert_eq!(
            account_row_hint(false, false, true, true, String::new()),
            "Ready to apply"
        );
    }

    #[test]
    fn accounts_paths_preserve_only_session_routes() {
        assert_eq!(
            accounts_path_with_return_to("/app/sessions/s%2F1"),
            "/app/accounts/?return_to=%2Fapp%2Fsessions%2Fs%252F1"
        );
        assert_eq!(
            accounts_back_to_chat_path("?return_to=%2Fapp%2Fsessions%2Fs%252F1"),
            "/app/sessions/s%2F1"
        );
        assert_eq!(accounts_back_to_chat_path("?return_to=%2Fapp%2F"), "/app/");
        assert_eq!(
            accounts_back_to_chat_path("?return_to=https%3A%2F%2Fexample.com"),
            "/app/"
        );
    }

    #[test]
    fn query_param_and_sign_out_visibility_helpers_match_accounts_routes() {
        assert_eq!(
            query_param("?return_to=%2Fapp%2Fsessions%2Fabc&x=1", "return_to"),
            Some("/app/sessions/abc".to_string())
        );
        assert_eq!(query_param("?x=1", "return_to"), None);
        assert!(accounts_page_shows_sign_out(Some(
            AccountsRouteAccess::Forbidden
        )));
        assert!(!accounts_page_shows_sign_out(Some(
            AccountsRouteAccess::SignInRequired
        )));
        assert_eq!(sign_out_button_label(false), "Sign out");
        assert_eq!(sign_out_button_label(true), "Signing out…");
    }

    #[test]
    fn accounts_page_shows_sign_out_for_admin_and_none() {
        let admin = sample_account("admin", true);
        assert!(accounts_page_shows_sign_out(Some(
            AccountsRouteAccess::Admin(admin)
        )));
        assert!(!accounts_page_shows_sign_out(None));
        assert!(!accounts_page_shows_sign_out(Some(
            AccountsRouteAccess::RegisterRequired
        )));
    }

    #[test]
    fn save_and_delete_button_labels_toggle_with_in_progress_state() {
        assert_eq!(create_account_button_label(false), "Create account");
        assert_eq!(create_account_button_label(true), "Saving…");
        assert_eq!(save_button_label(false), "Save");
        assert_eq!(save_button_label(true), "Saving…");
        assert_eq!(delete_button_label(false), "Delete");
        assert_eq!(delete_button_label(true), "Deleting…");
    }

    #[test]
    fn admin_access_label_reflects_admin_flag() {
        assert_eq!(admin_access_label(true), "Enabled");
        assert_eq!(admin_access_label(false), "Standard");
    }

    #[test]
    fn account_count_label_handles_zero_one_and_many() {
        assert_eq!(account_count_label(0), "0 accounts");
        assert_eq!(account_count_label(1), "1 account");
        assert_eq!(account_count_label(5), "5 accounts");
    }

    #[test]
    fn query_param_returns_none_for_missing_key_and_empty_search() {
        assert_eq!(query_param("", "return_to"), None);
        assert_eq!(query_param("?a=1&b=2", "return_to"), None);
        assert_eq!(
            query_param("return_to=%2Fapp%2F", "return_to"),
            Some("/app/".to_string())
        );
    }

    #[test]
    fn accounts_page_state_starts_with_empty_defaults() {
        let owner = Owner::new();
        owner.with(|| {
            let state = AccountsPageState::new();
            assert!(state.error.get().is_none());
            assert!(state.notice.get().is_none());
            assert!(state.access.get().is_none());
            assert!(state.current_user_id.get().is_empty());
            assert!(state.accounts.get().is_empty());
            assert!(state.loading_accounts.get());
            assert!(state.create_username.get().is_empty());
            assert!(state.create_password.get().is_empty());
            assert!(!state.create_admin.get());
            assert!(!state.creating.get());
            assert!(!state.checked.get());
        });
    }

    #[test]
    fn accounts_page_content_builds_for_each_access_state() {
        let owner = Owner::new();
        owner.with(|| {
            let state = AccountsPageState::new();
            let admin = sample_account("admin", true);

            state.access.set(Some(AccountsRouteAccess::Admin(admin)));
            let _ = view! { <AccountsPageContent state=state /> };

            state
                .access
                .set(Some(AccountsRouteAccess::RegisterRequired));
            let _ = view! { <AccountsPageContent state=state /> };

            state.access.set(Some(AccountsRouteAccess::SignInRequired));
            let _ = view! { <AccountsPageContent state=state /> };

            state.access.set(Some(AccountsRouteAccess::Forbidden));
            let _ = view! { <AccountsPageContent state=state /> };
        });
    }

    #[test]
    fn account_sections_and_rows_build_without_panicking() {
        let owner = Owner::new();
        owner.with(|| {
            let state = AccountsPageState::new();
            state.current_user_id.set("admin".to_string());
            state.accounts.set(vec![
                sample_account("admin", true),
                sample_account("member", false),
            ]);
            state.loading_accounts.set(false);

            let _ = view! { <CreateAccountSection state=state /> };
            let _ = view! { <CurrentAccountsSection state=state /> };
            let _ = view! { <AccountRow account=sample_account("member", false) state=state /> };
        });
    }

    #[test]
    fn account_row_subcomponents_build_without_panicking() {
        let owner = Owner::new();
        owner.with(|| {
            let password = RwSignal::new(String::new());
            let admin_checked = RwSignal::new(true);
            let saving = RwSignal::new(false);
            let deleting = RwSignal::new(false);
            let row_dirty = Signal::derive(|| true);
            let can_modify = Signal::derive(|| true);
            let role_label = Signal::derive(|| "admin".to_string());
            let role_badge_class =
                Signal::derive(|| "account-role-pill account-role-pill--admin".to_string());
            let constraint_label = Signal::derive(String::new);

            let _ = view! {
                <AccountRowSummary username="admin".to_string() constraint_label=constraint_label />
            };

            let _ = view! {
                <AccountStateCell
                    role_label=role_label
                    role_badge_class=role_badge_class
                />
            };

            let _ =
                view! { <AccountPasswordField password=password username="admin".to_string() /> };

            let _ =
                view! { <AccountAdminToggle admin_checked=admin_checked can_modify=can_modify /> };

            let _ = view! {
                <AccountRowActions
                    saving=saving
                    deleting=deleting
                    row_dirty=row_dirty
                    can_modify=can_modify
                    save_account=Callback::new(|()| {})
                    delete_account=Callback::new(|_: web_sys::MouseEvent| {})
                    constraint_label=constraint_label
                />
            };
        });
    }

    #[test]
    fn accounts_page_content_builds_waiting_and_loading_states() {
        let owner = Owner::new();
        owner.with(|| {
            let state = AccountsPageState::new();
            let _ = view! { <AccountsPageContent state=state /> };
            let _ = view! { <CurrentAccountsSection state=state /> };
        });
    }

    // -----------------------------------------------------------------------
    // account_row_hint – missing saving / deleting branches (pure)
    // -----------------------------------------------------------------------

    #[test]
    fn account_row_hint_saving_shows_saving_progress_message() {
        assert_eq!(
            account_row_hint(true, false, true, true, String::new()),
            "Saving changes…"
        );
    }

    #[test]
    fn account_row_hint_deleting_shows_removing_progress_message() {
        assert_eq!(
            account_row_hint(false, true, true, true, String::new()),
            "Removing account…"
        );
    }

    #[test]
    fn account_row_hint_not_modifiable_without_constraint_falls_through_to_ready() {
        assert_eq!(
            account_row_hint(false, false, true, false, String::new()),
            "Ready to apply"
        );
    }

    // -----------------------------------------------------------------------
    // account_row_dirty_signal – derived signal closure (signal-based)
    // -----------------------------------------------------------------------

    #[test]
    fn account_row_dirty_signal_reflects_password_and_admin_changes() {
        let owner = Owner::new();
        owner.with(|| {
            let account = sample_account("user", false);
            let password = RwSignal::new(String::new());
            let admin_checked = RwSignal::new(false);

            let dirty = account_row_dirty_signal(&account, password, admin_checked);
            assert!(!dirty.get());

            password.set("new_pass".to_string());
            assert!(dirty.get());

            password.set("   ".to_string());
            assert!(!dirty.get());

            password.set(String::new());
            admin_checked.set(true);
            assert!(dirty.get());
        });
    }

    // -----------------------------------------------------------------------
    // account_capabilities_signal – derived signal closure (signal-based)
    // -----------------------------------------------------------------------

    #[test]
    fn account_capabilities_signal_evaluates_against_state_accounts() {
        let owner = Owner::new();
        owner.with(|| {
            let state = AccountsPageState::new();
            let account = sample_account("user1", false);
            state.current_user_id.set("admin".to_string());
            state
                .accounts
                .set(vec![sample_account("admin", true), account.clone()]);

            let caps = account_capabilities_signal(&account, state);
            assert!(caps.get().constraint.is_none());
            assert!(caps.get().can_modify());
        });
    }

    // -----------------------------------------------------------------------
    // account_role_kind_signal + account_role_label_signal (signal-based)
    // -----------------------------------------------------------------------

    #[test]
    fn account_role_kind_signal_and_label_signal_evaluate_correctly() {
        let owner = Owner::new();
        owner.with(|| {
            let state = AccountsPageState::new();
            let account = sample_account("alice", true);
            state.current_user_id.set("bob".to_string());

            let role_kind = account_role_kind_signal(&account, state);
            assert_eq!(role_kind.get(), AccountRoleKind::Admin);

            let label = account_role_label_signal(role_kind);
            assert_eq!(label.get(), "admin");
        });
    }

    // -----------------------------------------------------------------------
    // account_role_badge_class_signal – derived signal closure (signal-based)
    // -----------------------------------------------------------------------

    #[test]
    fn account_role_badge_class_signal_formats_css_modifier_correctly() {
        let owner = Owner::new();
        owner.with(|| {
            let kind = RwSignal::new(AccountRoleKind::Member);
            let kind_sig = Signal::derive(move || kind.get());
            let badge = account_role_badge_class_signal(kind_sig);
            assert!(badge.get().contains("member"));

            kind.set(AccountRoleKind::Active);
            assert!(badge.get().contains("active"));
        });
    }

    // -----------------------------------------------------------------------
    // account_constraint_label_signal + account_can_modify_signal (signal-based)
    // -----------------------------------------------------------------------

    #[test]
    fn account_constraint_label_signal_shows_last_admin_message() {
        let owner = Owner::new();
        owner.with(|| {
            let state = AccountsPageState::new();
            let account = sample_account("sole_admin", true);
            state.current_user_id.set("other".to_string());
            state.accounts.set(vec![account.clone()]);

            let caps = account_capabilities_signal(&account, state);
            let label = account_constraint_label_signal(caps);
            assert_eq!(label.get(), "One admin account must remain");
        });
    }

    #[test]
    fn account_can_modify_signal_false_for_constrained_account() {
        let owner = Owner::new();
        owner.with(|| {
            let state = AccountsPageState::new();
            let account = sample_account("sole_admin", true);
            state.current_user_id.set("other".to_string());
            state.accounts.set(vec![account.clone()]);

            let caps = account_capabilities_signal(&account, state);
            let can_modify = account_can_modify_signal(caps);
            assert!(!can_modify.get());
        });
    }

    #[test]
    fn account_row_hint_signal_tracks_signal_inputs() {
        let owner = Owner::new();
        owner.with(|| {
            let saving = RwSignal::new(false);
            let deleting = RwSignal::new(false);
            let row_dirty_value = RwSignal::new(false);
            let can_modify_value = RwSignal::new(true);
            let constraint = RwSignal::new(String::new());
            let row_dirty = Signal::derive(move || row_dirty_value.get());
            let can_modify = Signal::derive(move || can_modify_value.get());
            let constraint_label = Signal::derive(move || constraint.get());
            let hint =
                account_row_hint_signal(saving, deleting, row_dirty, can_modify, constraint_label);

            assert_eq!(hint.get(), "No pending changes");

            row_dirty_value.set(true);
            can_modify_value.set(false);
            constraint.set("One admin account must remain".to_string());
            assert_eq!(hint.get(), "One admin account must remain");

            saving.set(true);
            assert_eq!(hint.get(), "Saving changes…");
        });
    }

    // -----------------------------------------------------------------------
    // account_row_action_handlers – construction (signal-based)
    // -----------------------------------------------------------------------

    #[test]
    fn account_row_action_handlers_creates_both_save_and_delete_callbacks() {
        let owner = Owner::new();
        owner.with(|| {
            let state = AccountsPageState::new();
            let account = sample_account("user1", false);
            let password = RwSignal::new(String::new());
            let admin_checked = RwSignal::new(false);
            let saving = RwSignal::new(false);
            let deleting = RwSignal::new(false);

            let (save_cb, delete_cb) = account_row_action_handlers(
                &account,
                state,
                password,
                admin_checked,
                saving,
                deleting,
            );
            let _ = (save_cb, delete_cb);
        });
    }

    #[test]
    fn session_sidebar_auth_controls_and_accounts_page_render_host_safe_views() {
        let owner = Owner::new();
        owner.with(|| {
            let error = RwSignal::new(None::<String>);
            let _ = view! {
                <SessionSidebarAuthControls current_session_id="session-1".to_string() error=error />
            };

            let _ = view! { <AccountsPage /> };
        });
    }

    #[test]
    fn helper_views_render_admin_and_notice_states() {
        let owner = Owner::new();
        owner.with(|| {
            let is_admin = RwSignal::new(true);
            let signed_in = RwSignal::new(true);
            let signing_out = RwSignal::new(false);
            let _ = session_sidebar_auth_controls_view(
                "/app/accounts/?return_to=%2Fapp%2Fsessions%2Fabc".to_string(),
                is_admin,
                signed_in,
                signing_out,
                Callback::new(|_: web_sys::MouseEvent| {}),
            );

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
            );
        });
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn host_initializers_and_handlers_update_state_safely() {
        let owner = Owner::new();
        owner.with(|| {
            let checked = RwSignal::new(false);
            initialize_session_sidebar_auth_controls_host(checked);
            assert!(checked.get());
            initialize_session_sidebar_auth_controls_host(checked);
            assert!(checked.get());

            let state = AccountsPageState::new();
            initialize_accounts_page_host(state);
            assert!(state.checked.get());
            assert!(!state.loading_accounts.get());
            state.loading_accounts.set(true);
            initialize_accounts_page_host(state);
            assert!(state.loading_accounts.get());

            state.create_username.set("alice".to_string());
            state.create_password.set("password123".to_string());
            state.create_admin.set(true);
            create_account_submit_host(state);
            assert!(!state.creating.get());
            assert!(state.create_username.get().is_empty());
            assert!(state.create_password.get().is_empty());
            assert!(!state.create_admin.get());
            assert_eq!(state.notice.get(), Some("Account created.".to_string()));

            let password = RwSignal::new("next-password".to_string());
            let admin_checked = RwSignal::new(true);
            let saving = RwSignal::new(false);
            account_save_host("alice", state, password, admin_checked, saving);
            assert!(!saving.get());
            assert!(password.get().is_empty());
            assert_eq!(state.notice.get(), Some("Account updated.".to_string()));

            let deleting = RwSignal::new(false);
            account_delete_host("alice", state, deleting);
            assert!(!deleting.get());
            assert_eq!(state.notice.get(), Some("Account deleted.".to_string()));

            state.error.set(Some("stale error".to_string()));
            let signing_out = RwSignal::new(false);
            sign_out_host(state.error, signing_out);
            assert!(signing_out.get());
            assert!(state.error.get().is_none());

            state.error.set(Some("reload error".to_string()));
            state.loading_accounts.set(true);
            spawn_account_reload(state);
            assert!(!state.loading_accounts.get());
            assert!(state.error.get().is_none());

            assert_eq!(accounts_back_to_chat_path_from_location(), "/app/");
        });
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn host_callbacks_and_helpers_leave_in_progress_state_unchanged() {
        let owner = Owner::new();
        owner.with(|| {
            let state = AccountsPageState::new();

            state.creating.set(true);
            state.notice.set(Some("still creating".to_string()));
            create_account_submit_host(state);
            assert_eq!(state.notice.get(), Some("still creating".to_string()));

            let password = RwSignal::new("unchanged".to_string());
            let admin_checked = RwSignal::new(true);
            let saving = RwSignal::new(true);
            state.notice.set(Some("still saving".to_string()));
            account_save_host("alice", state, password, admin_checked, saving);
            assert_eq!(password.get(), "unchanged");
            assert_eq!(state.notice.get(), Some("still saving".to_string()));

            let deleting = RwSignal::new(true);
            state.notice.set(Some("still deleting".to_string()));
            account_delete_host("alice", state, deleting);
            assert_eq!(state.notice.get(), Some("still deleting".to_string()));

            state.error.set(Some("still signing out".to_string()));
            let signing_out = RwSignal::new(true);
            sign_out_host(state.error, signing_out);
            assert_eq!(state.error.get(), Some("still signing out".to_string()));

            create_account_submit_handler(state)(fake_submit_event());
            account_save_handler(
                "alice".to_string(),
                state,
                RwSignal::new(String::new()),
                RwSignal::new(false),
                RwSignal::new(false),
            )
            .run(());
            account_delete_handler("alice".to_string(), state, RwSignal::new(false))
                .run(fake_mouse_event());
            sign_out_handler(state.error, RwSignal::new(false)).run(fake_mouse_event());
        });
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn event_target_checked_returns_false_on_host() {
        assert!(!super::event_target_checked(&()));
    }
}
