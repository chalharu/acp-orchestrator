use acp_contracts::LocalAccount;
use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::{
    application::auth::{account_capabilities, accounts_route_access},
    components::ErrorBanner,
    domain::auth::AccountsRouteAccess,
    infrastructure::api,
};

#[component]
pub fn SessionSidebarAccountsLink() -> impl IntoView {
    let is_admin = RwSignal::new(false);
    let checked = RwSignal::new(false);

    Effect::new(move |_| {
        if checked.get() {
            return;
        }
        checked.set(true);
        leptos::task::spawn_local(async move {
            if let Ok(status) = api::auth_status().await {
                is_admin.set(status.account.is_some_and(|account| account.is_admin));
            }
        });
    });

    view! {
        <Show when=move || is_admin.get()>
            <a class="session-sidebar__secondary-link" href="/app/accounts/">
                "Accounts"
            </a>
        </Show>
    }
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
    initialize_accounts_page(state);

    view! {
        <main class="app-shell account-shell">
            <ErrorBanner message=state.error />
            <section class="panel account-panel">
                <div class="account-panel__header">
                    <h1>"Accounts"</h1>
                    <a href="/app/">"Back to chat"</a>
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

fn initialize_accounts_page(state: AccountsPageState) {
    Effect::new(move |_| {
        if state.checked.get() {
            return;
        }

        state.checked.set(true);
        leptos::task::spawn_local(async move {
            match api::auth_status().await {
                Ok(status) => {
                    let access = accounts_route_access(&status);
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

#[component]
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
fn CreateAccountSection(state: AccountsPageState) -> impl IntoView {
    let on_submit = create_account_submit_handler(state);

    view! {
        <div class="account-panel__section">
            <h2>"Create account"</h2>
            <form class="account-form" on:submit=on_submit>
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

#[component]
fn CurrentAccountsSection(state: AccountsPageState) -> impl IntoView {
    view! {
        <div class="account-panel__section">
            <h2>"Current accounts"</h2>
            <Show
                when=move || !state.loading_accounts.get()
                fallback=|| view! { <p class="muted">"Loading accounts…"</p> }
            >
                <ul class="account-list">
                    <For
                        each=move || state.accounts.get()
                        key=|account| account.user_id.clone()
                        children=move |account| view! { <AccountRow account state /> }
                    />
                </ul>
            </Show>
        </div>
    }
}

#[component]
fn AccountRow(account: LocalAccount, state: AccountsPageState) -> impl IntoView {
    let password = RwSignal::new(String::new());
    let admin_checked = RwSignal::new(account.is_admin);
    let saving = RwSignal::new(false);
    let deleting = RwSignal::new(false);
    let capabilities_account = account.clone();
    let capabilities = Signal::derive(move || {
        account_capabilities(
            &state.current_user_id.get(),
            &state.accounts.get(),
            &capabilities_account,
        )
    });
    let save_account = account_save_handler(
        account.user_id.clone(),
        state,
        password,
        admin_checked,
        saving,
    );
    let delete_account = account_delete_handler(account.user_id.clone(), state, deleting);

    view! {
        <li class="account-list__item">
            <form class="account-row" on:submit=move |event| save_account.run(event)>
                <div class="account-row__summary">
                    <strong>{account.username.clone()}</strong>
                    <span class="muted">{account_role_label(&account, &state.current_user_id.get_untracked())}</span>
                </div>
                <label class="account-form__field">
                    <span>"New password"</span>
                    <input
                        type="password"
                        prop:value=move || password.get()
                        on:input=move |event| password.set(event_target_value(&event))
                    />
                </label>
                <label class="account-checkbox">
                    <input
                        type="checkbox"
                        prop:checked=move || admin_checked.get()
                        prop:disabled=move || !capabilities.get().can_toggle_admin
                        on:change=move |event| admin_checked.set(event_target_checked(&event))
                    />
                    <span>"Admin"</span>
                </label>
                <div class="account-row__actions">
                    <button type="submit" prop:disabled=move || saving.get()>
                        {move || if saving.get() { "Saving…" } else { "Save" }}
                    </button>
                    <button
                        type="button"
                        class="account-row__delete"
                        prop:disabled=move || deleting.get() || !capabilities.get().can_delete
                        on:click=move |event| delete_account.run(event)
                    >
                        {move || if deleting.get() { "Deleting…" } else { "Delete" }}
                    </button>
                </div>
            </form>
        </li>
    }
}

fn account_save_handler(
    account_user_id: String,
    state: AccountsPageState,
    password: RwSignal<String>,
    admin_checked: RwSignal<bool>,
    saving: RwSignal<bool>,
) -> Callback<web_sys::SubmitEvent> {
    Callback::new(move |event: web_sys::SubmitEvent| {
        event.prevent_default();
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

fn password_update(password: String) -> Option<String> {
    if password.trim().is_empty() {
        None
    } else {
        Some(password)
    }
}

fn account_role_label(account: &LocalAccount, current_user_id: &str) -> String {
    if account.user_id == current_user_id {
        "signed in".to_string()
    } else if account.is_admin {
        "admin".to_string()
    } else {
        "member".to_string()
    }
}

fn event_target_checked(event: &web_sys::Event) -> bool {
    event
        .target()
        .and_then(|target| target.dyn_into::<web_sys::HtmlInputElement>().ok())
        .map(|input| input.checked())
        .unwrap_or(false)
}

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

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

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
    fn password_update_ignores_blank_passwords() {
        assert_eq!(password_update("   ".to_string()), None);
        assert_eq!(
            password_update("password123".to_string()),
            Some("password123".to_string())
        );
    }

    #[test]
    fn account_role_label_prefers_signed_in_over_admin() {
        let admin = sample_account("admin", true);
        let member = sample_account("member", false);

        assert_eq!(account_role_label(&admin, "admin"), "signed in");
        assert_eq!(account_role_label(&admin, "other"), "admin");
        assert_eq!(account_role_label(&member, "other"), "member");
    }
}
