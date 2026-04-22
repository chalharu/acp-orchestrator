use leptos::prelude::*;

use crate::{
    application::auth::AccountsRouteAccess,
    components::ErrorBanner,
};

use super::{
    create_account::CreateAccountSection,
    registry::CurrentAccountsSection,
    shared::{
        AccountsPageState, accounts_back_to_chat_path_from_location,
        accounts_page_shows_sign_out, initialize_accounts_page, sign_out_button_label,
        sign_out_handler,
    },
};

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
                on:click=move |event| sign_out.run(event)
                prop:disabled=signing_out
            >
                {label}
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

#[component]
fn AccountsPageContent(state: AccountsPageState) -> impl IntoView {
    accounts_page_content(state)
}

#[cfg(target_family = "wasm")]
fn accounts_page_content(state: AccountsPageState) -> impl IntoView {
    move || accounts_page_content_body(state.access.get(), state)
}

#[cfg(not(target_family = "wasm"))]
fn accounts_page_content(state: AccountsPageState) -> impl IntoView {
    accounts_page_content_body(state.access.get_untracked(), state)
}

fn accounts_page_content_body(
    access: Option<AccountsRouteAccess>,
    state: AccountsPageState,
) -> AnyView {
    match access {
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
            );
            let _ = view! { <AccountsPage /> };
        });
    }
}
