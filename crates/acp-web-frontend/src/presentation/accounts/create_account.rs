#![cfg_attr(not(target_family = "wasm"), allow(dead_code))]

use leptos::prelude::*;

#[cfg(target_family = "wasm")]
use crate::infrastructure::api;

use super::shared::{AccountsPageState, event_target_checked, spawn_account_reload};

#[component]
#[cfg(target_family = "wasm")]
pub(super) fn CreateAccountSection(state: AccountsPageState) -> impl IntoView {
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
                    {move || create_account_button_label(state.creating.get())}
                </button>
            </form>
        </div>
    }
}

#[component]
#[cfg(not(target_family = "wasm"))]
pub(super) fn CreateAccountSection(state: AccountsPageState) -> impl IntoView {
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

#[cfg(test)]
mod tests {
    use leptos::prelude::*;

    use super::*;
    use crate::presentation::accounts::shared::AccountsPageState;
    use wasm_bindgen::{JsCast, JsValue};

    #[cfg(not(target_family = "wasm"))]
    fn fake_submit_event() -> web_sys::SubmitEvent {
        JsValue::NULL.unchecked_into()
    }

    #[test]
    fn create_account_button_label_toggles_with_in_progress_state() {
        assert_eq!(create_account_button_label(false), "Create account");
        assert_eq!(create_account_button_label(true), "Saving…");
    }

    #[test]
    fn create_account_section_builds_without_panicking() {
        let owner = Owner::new();
        owner.with(|| {
            let state = AccountsPageState::new();
            let _ = view! { <CreateAccountSection state=state /> };
        });
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn create_account_submit_host_resets_form_state_and_sets_notice() {
        let owner = Owner::new();
        owner.with(|| {
            let state = AccountsPageState::new();
            state.create_username.set("alice".to_string());
            state.create_password.set("password123".to_string());
            state.create_admin.set(true);
            create_account_submit_host(state);
            assert!(!state.creating.get());
            assert!(state.create_username.get().is_empty());
            assert!(state.create_password.get().is_empty());
            assert!(!state.create_admin.get());
            assert_eq!(state.notice.get(), Some("Account created.".to_string()));
        });
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn host_callbacks_leave_in_progress_state_unchanged() {
        let owner = Owner::new();
        owner.with(|| {
            let state = AccountsPageState::new();
            state.creating.set(true);
            state.notice.set(Some("still creating".to_string()));
            create_account_submit_host(state);
            assert_eq!(state.notice.get(), Some("still creating".to_string()));
            create_account_submit_handler(state)(fake_submit_event());
        });
    }
}
