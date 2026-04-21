use leptos::prelude::*;

use crate::{
    application::auth::home_route_target, components::ErrorBanner, domain::auth::HomeRouteTarget,
    infrastructure::api, navigate_to,
};

#[component]
pub fn RegisterPage() -> impl IntoView {
    let username = RwSignal::new(String::new());
    let password = RwSignal::new(String::new());
    let error = RwSignal::new(None::<String>);
    let loading = RwSignal::new(true);
    let submitting = RwSignal::new(false);
    let checked = RwSignal::new(false);

    Effect::new(move |_| {
        if checked.get() {
            return;
        }
        checked.set(true);
        leptos::task::spawn_local(async move {
            match api::auth_status().await {
                Ok(status) => match home_route_target(&status) {
                    HomeRouteTarget::PrepareSession => {
                        let _ = navigate_to("/app/");
                    }
                    HomeRouteTarget::Register => loading.set(false),
                    HomeRouteTarget::SignIn => {
                        let _ = navigate_to("/app/sign-in/");
                    }
                },
                Err(message) => {
                    loading.set(false);
                    error.set(Some(message));
                }
            }
        });
    });

    let on_submit = move |event: web_sys::SubmitEvent| {
        event.prevent_default();
        if submitting.get_untracked() {
            return;
        }
        submitting.set(true);
        error.set(None);
        let username_value = username.get_untracked();
        let password_value = password.get_untracked();
        leptos::task::spawn_local(async move {
            match api::bootstrap_register(&username_value, &password_value).await {
                Ok(_) => {
                    let _ = navigate_to("/app/");
                }
                Err(message) => {
                    submitting.set(false);
                    error.set(Some(message));
                }
            }
        });
    };

    view! {
        <main class="app-shell app-shell--home">
            <ErrorBanner message=error />
            <section class="panel account-panel">
                <h1>"Register bootstrap account"</h1>
                <Show
                    when=move || !loading.get()
                    fallback=|| view! { <p class="muted">"Checking registration status…"</p> }
                >
                    <form class="account-form" on:submit=on_submit>
                        <label class="account-form__field">
                            <span>"User name"</span>
                            <input
                                type="text"
                                prop:value=move || username.get()
                                on:input=move |event| username.set(event_target_value(&event))
                                autocomplete="username"
                            />
                        </label>
                        <label class="account-form__field">
                            <span>"Password"</span>
                            <input
                                type="password"
                                prop:value=move || password.get()
                                on:input=move |event| password.set(event_target_value(&event))
                                autocomplete="new-password"
                            />
                        </label>
                        <button
                            type="submit"
                            class="account-form__submit"
                            prop:disabled=move || submitting.get()
                        >
                            {move || if submitting.get() { "Creating account…" } else { "Create account" }}
                        </button>
                    </form>
                </Show>
            </section>
        </main>
    }
}
