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

#[component]
pub fn AccountsPage() -> impl IntoView {
    let error = RwSignal::new(None::<String>);
    let notice = RwSignal::new(None::<String>);
    let access = RwSignal::new(None::<AccountsRouteAccess>);
    let current_user_id = RwSignal::new(String::new());
    let accounts = RwSignal::new(Vec::<LocalAccount>::new());
    let loading_accounts = RwSignal::new(true);
    let create_username = RwSignal::new(String::new());
    let create_password = RwSignal::new(String::new());
    let create_admin = RwSignal::new(false);
    let creating = RwSignal::new(false);
    let checked = RwSignal::new(false);

    Effect::new(move |_| {
        if checked.get() {
            return;
        }
        checked.set(true);
        leptos::task::spawn_local(async move {
            match api::auth_status().await {
                Ok(status) => {
                    let next_access = accounts_route_access(&status);
                    let should_load = matches!(next_access, AccountsRouteAccess::Admin(_));
                    access.set(Some(next_access));
                    if should_load {
                        spawn_account_reload(current_user_id, accounts, loading_accounts, error);
                    } else {
                        loading_accounts.set(false);
                    }
                }
                Err(message) => {
                    loading_accounts.set(false);
                    error.set(Some(message));
                }
            }
        });
    });

    let create_account = move |event: web_sys::SubmitEvent| {
        event.prevent_default();
        if creating.get_untracked() {
            return;
        }
        creating.set(true);
        error.set(None);
        notice.set(None);
        let username = create_username.get_untracked();
        let password = create_password.get_untracked();
        let is_admin = create_admin.get_untracked();
        leptos::task::spawn_local(async move {
            match api::create_account(&username, &password, is_admin).await {
                Ok(_) => {
                    create_username.set(String::new());
                    create_password.set(String::new());
                    create_admin.set(false);
                    notice.set(Some("Account created.".to_string()));
                    creating.set(false);
                    spawn_account_reload(current_user_id, accounts, loading_accounts, error);
                }
                Err(message) => {
                    creating.set(false);
                    error.set(Some(message));
                }
            }
        });
    };

    view! {
        <main class="app-shell account-shell">
            <ErrorBanner message=error />
            <section class="panel account-panel">
                <div class="account-panel__header">
                    <h1>"Accounts"</h1>
                    <a href="/app/">"Back to chat"</a>
                </div>
                <Show when=move || notice.get().is_some()>
                    <p class="account-notice" role="status">
                        {move || notice.get().unwrap_or_default()}
                    </p>
                </Show>
                {move || match access.get() {
                    Some(AccountsRouteAccess::Admin(_)) => view! {
                        <div class="account-panel__section">
                            <h2>"Create account"</h2>
                            <form class="account-form" on:submit=create_account>
                                <label class="account-form__field">
                                    <span>"User name"</span>
                                    <input
                                        type="text"
                                        prop:value=move || create_username.get()
                                        on:input=move |event| create_username.set(event_target_value(&event))
                                    />
                                </label>
                                <label class="account-form__field">
                                    <span>"Password"</span>
                                    <input
                                        type="password"
                                        prop:value=move || create_password.get()
                                        on:input=move |event| create_password.set(event_target_value(&event))
                                    />
                                </label>
                                <label class="account-checkbox">
                                    <input
                                        type="checkbox"
                                        prop:checked=move || create_admin.get()
                                        on:change=move |event| create_admin.set(event_target_checked(&event))
                                    />
                                    <span>"Admin"</span>
                                </label>
                                <button
                                    type="submit"
                                    class="account-form__submit"
                                    prop:disabled=move || creating.get()
                                >
                                    {move || if creating.get() { "Saving…" } else { "Create account" }}
                                </button>
                            </form>
                        </div>
                        <div class="account-panel__section">
                            <h2>"Current accounts"</h2>
                            <Show
                                when=move || !loading_accounts.get()
                                fallback=|| view! { <p class="muted">"Loading accounts…"</p> }
                            >
                                <ul class="account-list">
                                    <For
                                        each=move || accounts.get()
                                        key=|account| account.user_id.clone()
                                        children=move |account| {
                                            let password = RwSignal::new(String::new());
                                            let admin_checked = RwSignal::new(account.is_admin);
                                            let saving = RwSignal::new(false);
                                            let deleting = RwSignal::new(false);
                                            let current_user_id = current_user_id;
                                            let accounts = accounts;
                                            let error = error;
                                            let notice = notice;
                                            let capabilities_account = account.clone();
                                            let capabilities = Signal::derive(move || {
                                                account_capabilities(
                                                    &current_user_id.get(),
                                                    &accounts.get(),
                                                    &capabilities_account,
                                                )
                                            });

                                            let save_account = {
                                                let account_user_id = account.user_id.clone();
                                                move |event: web_sys::SubmitEvent| {
                                                    event.prevent_default();
                                                    if saving.get_untracked() {
                                                        return;
                                                    }
                                                        saving.set(true);
                                                        error.set(None);
                                                        notice.set(None);
                                                        let password_value = password.get_untracked();
                                                        let password_update = if password_value.trim().is_empty() {
                                                            None
                                                    } else {
                                                        Some(password_value)
                                                    };
                                                    let is_admin = Some(admin_checked.get_untracked());
                                                    let account_user_id = account_user_id.clone();
                                                    leptos::task::spawn_local(async move {
                                                            match api::update_account(&account_user_id, password_update, is_admin).await {
                                                                Ok(_) => {
                                                                    password.set(String::new());
                                                                    saving.set(false);
                                                                    notice.set(Some("Account updated.".to_string()));
                                                                spawn_account_reload(
                                                                    current_user_id,
                                                                    accounts,
                                                                    loading_accounts,
                                                                    error,
                                                                );
                                                            }
                                                            Err(message) => {
                                                                saving.set(false);
                                                                error.set(Some(message));
                                                            }
                                                        }
                                                    });
                                                }
                                            };

                                            let delete_account = {
                                                let account_user_id = account.user_id.clone();
                                                move |_| {
                                                    if deleting.get_untracked() {
                                                        return;
                                                    }
                                                    deleting.set(true);
                                                    error.set(None);
                                                    notice.set(None);
                                                    let account_user_id = account_user_id.clone();
                                                    leptos::task::spawn_local(async move {
                                                        match api::delete_account(&account_user_id).await {
                                                            Ok(_) => {
                                                                deleting.set(false);
                                                                notice.set(Some("Account deleted.".to_string()));
                                                                spawn_account_reload(
                                                                    current_user_id,
                                                                    accounts,
                                                                    loading_accounts,
                                                                    error,
                                                                );
                                                            }
                                                            Err(message) => {
                                                                deleting.set(false);
                                                                error.set(Some(message));
                                                            }
                                                        }
                                                    });
                                                }
                                            };

                                            view! {
                                                <li class="account-list__item">
                                                    <form class="account-row" on:submit=save_account>
                                                        <div class="account-row__summary">
                                                            <strong>{account.username.clone()}</strong>
                                                            <span class="muted">
                                                                {if account.user_id == current_user_id.get_untracked() {
                                                                    "signed in".to_string()
                                                                } else if account.is_admin {
                                                                    "admin".to_string()
                                                                } else {
                                                                    "member".to_string()
                                                                }}
                                                            </span>
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
                                                                on:click=delete_account
                                                            >
                                                                {move || if deleting.get() { "Deleting…" } else { "Delete" }}
                                                            </button>
                                                        </div>
                                                    </form>
                                                </li>
                                            }
                                        }
                                    />
                                </ul>
                            </Show>
                        </div>
                    }.into_any(),
                    Some(AccountsRouteAccess::RegisterRequired) => view! {
                        <p class="muted">
                            "Bootstrap registration is still required. "
                            <a href="/app/register/">"Create the first account."</a>
                        </p>
                    }.into_any(),
                    Some(AccountsRouteAccess::SignInRequired) => view! {
                        <p class="muted">
                            "Sign in is required before managing accounts. "
                            <a href="/app/sign-in/">"Open sign-in."</a>
                        </p>
                    }.into_any(),
                    Some(AccountsRouteAccess::Forbidden) => view! {
                        <p class="muted">"This page is available only to admin accounts."</p>
                    }.into_any(),
                    None => view! { <p class="muted">"Checking account access…"</p> }.into_any(),
                }}
            </section>
        </main>
    }
}

fn event_target_checked(event: &web_sys::Event) -> bool {
    event
        .target()
        .and_then(|target| target.dyn_into::<web_sys::HtmlInputElement>().ok())
        .map(|input| input.checked())
        .unwrap_or(false)
}

fn spawn_account_reload(
    current_user_id: RwSignal<String>,
    accounts: RwSignal<Vec<LocalAccount>>,
    loading_accounts: RwSignal<bool>,
    error: RwSignal<Option<String>>,
) {
    loading_accounts.set(true);
    error.set(None);
    leptos::task::spawn_local(async move {
        match api::list_accounts().await {
            Ok(response) => {
                current_user_id.set(response.current_user_id);
                accounts.set(response.accounts);
                loading_accounts.set(false);
            }
            Err(message) => {
                loading_accounts.set(false);
                error.set(Some(message));
            }
        }
    });
}
