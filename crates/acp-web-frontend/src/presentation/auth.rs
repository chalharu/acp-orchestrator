use leptos::prelude::*;

use crate::{
    application::auth::{AuthSignals, submit_sign_in, submit_sign_up},
    components::ErrorBanner,
    domain::auth::sign_in_shows_registration_link,
};

#[component]
pub(crate) fn AuthLoadingPage(message: &'static str) -> impl IntoView {
    view! {
        <main class="app-shell app-shell--home">
            <section class="panel empty-state">
                <p class="muted">{message}</p>
            </section>
        </main>
    }
}

#[component]
pub(crate) fn SignInPage(auth: AuthSignals) -> impl IntoView {
    let sign_in_disabled = sign_in_disabled_signal(auth);
    let auth_busy = auth_request_busy_signal(auth);

    view! {
        <main class="app-shell app-shell--home">
            <ErrorBanner message=auth.error />
            <section class="panel auth-panel">
                {sign_in_panel_copy(auth)}
                {sign_in_form(auth, sign_in_disabled, auth_busy)}
            </section>
        </main>
    }
}

#[component]
pub(crate) fn RegisterPage(auth: AuthSignals) -> impl IntoView {
    let sign_up_disabled = sign_up_disabled_signal(auth);
    let auth_busy = auth_request_busy_signal(auth);

    view! {
        <main class="app-shell app-shell--home">
            <ErrorBanner message=auth.error />
            <section class="panel auth-panel">
                {register_panel_copy(auth)}
                <Show
                    when=move || auth.registration_notice.get().is_some()
                    fallback=|| ()
                >
                    <p class="muted">
                        {move || auth.registration_notice.get().unwrap_or_default()}
                    </p>
                </Show>
                {register_form(auth, sign_up_disabled, auth_busy)}
            </section>
        </main>
    }
}

#[component]
pub(crate) fn RegistrationUnavailablePage() -> impl IntoView {
    view! {
        <main class="app-shell app-shell--home">
            <section class="panel empty-state">
                <p class="muted">"Account creation is available only to administrators."</p>
                <p class="muted">
                    <a href="/app/">"Back to chats"</a>
                </p>
            </section>
        </main>
    }
}

fn auth_request_busy_signal(auth: AuthSignals) -> Signal<bool> {
    Signal::derive(move || auth.signing_in.get() || auth.signing_up.get())
}

fn sign_in_disabled_signal(auth: AuthSignals) -> Signal<bool> {
    Signal::derive(move || {
        auth.signing_in.get()
            || auth.signing_up.get()
            || auth.user_name_draft.get().trim().is_empty()
            || auth.password_draft.get().is_empty()
    })
}

fn sign_up_disabled_signal(auth: AuthSignals) -> Signal<bool> {
    Signal::derive(move || {
        auth.signing_in.get()
            || auth.signing_up.get()
            || auth.user_name_draft.get().trim().is_empty()
            || auth.password_draft.get().chars().count() < 8
    })
}

fn sign_in_panel_copy(auth: AuthSignals) -> impl IntoView {
    view! {
        <div class="auth-panel__copy">
            <p class="auth-panel__eyebrow">"ACP Web"</p>
            <h1 class="auth-panel__title">"Sign in"</h1>
            <p class="muted">
                "Enter your user name and password to access your browser workspace session."
            </p>
            <Show
                when=move || sign_in_shows_registration_link(auth.bootstrap_registration_open.get())
                fallback=|| ()
            >
                <p class="muted">
                    <a href="/app/register/">"Create an account"</a>
                </p>
            </Show>
        </div>
    }
}

fn register_panel_copy(auth: AuthSignals) -> impl IntoView {
    let admin_creating_user = move || {
        auth.session
            .get()
            .as_ref()
            .is_some_and(|session| session.is_admin)
    };

    view! {
        <div class="auth-panel__copy">
            <p class="auth-panel__eyebrow">"ACP Web"</p>
            <h1 class="auth-panel__title">"Create account"</h1>
            <p class="muted">
                {move || {
                    if admin_creating_user() {
                        "Admins can create additional local users."
                    } else {
                        "Choose a user name and a password with at least 8 characters."
                    }
                }}
            </p>
            <p class="muted">
                <a href="/app/">
                    {move || if admin_creating_user() { "Back to chats" } else { "Back to sign in" }}
                </a>
            </p>
        </div>
    }
}

fn sign_in_form(
    auth: AuthSignals,
    sign_in_disabled: Signal<bool>,
    auth_busy: Signal<bool>,
) -> impl IntoView {
    let button_label = Signal::derive(move || sign_in_button_label(auth.signing_in.get()));
    auth_credentials_form(
        auth,
        sign_in_disabled,
        auth_busy,
        button_label,
        "current-password",
        submit_sign_in,
    )
}

fn register_form(
    auth: AuthSignals,
    sign_up_disabled: Signal<bool>,
    auth_busy: Signal<bool>,
) -> impl IntoView {
    let button_label = Signal::derive(move || sign_up_button_label(auth.signing_up.get()));
    auth_credentials_form(
        auth,
        sign_up_disabled,
        auth_busy,
        button_label,
        "new-password",
        submit_sign_up,
    )
}

fn auth_credentials_form(
    auth: AuthSignals,
    submit_disabled: Signal<bool>,
    auth_busy: Signal<bool>,
    button_label: Signal<&'static str>,
    password_autocomplete: &'static str,
    on_submit: fn(AuthSignals),
) -> impl IntoView {
    view! {
        <form
            class="auth-form"
            on:submit=move |ev: web_sys::SubmitEvent| {
                ev.prevent_default();
                on_submit(auth);
            }
        >
            {auth_user_name_field(auth, auth_busy)}
            {auth_password_field(auth, auth_busy, password_autocomplete)}
            {auth_submit_button(submit_disabled, button_label)}
        </form>
    }
}

fn auth_user_name_field(auth: AuthSignals, auth_busy: Signal<bool>) -> impl IntoView {
    view! {
        <label class="auth-form__label" for="sign-in-user-name">
            "User name"
        </label>
        <input
            id="sign-in-user-name"
            class="auth-form__input"
            type="text"
            autofocus=true
            maxlength="100"
            autocomplete="username"
            prop:value=move || auth.user_name_draft.get()
            prop:disabled=move || auth_busy.get()
            on:input=move |ev| {
                auth.user_name_draft.set(event_target_value(&ev));
                auth.registration_notice.set(None);
            }
        />
    }
}

fn auth_password_field(
    auth: AuthSignals,
    auth_busy: Signal<bool>,
    autocomplete: &'static str,
) -> impl IntoView {
    view! {
        <label class="auth-form__label" for="sign-in-password">
            "Password"
        </label>
        <input
            id="sign-in-password"
            class="auth-form__input"
            type="password"
            autocomplete=autocomplete
            prop:value=move || auth.password_draft.get()
            prop:disabled=move || auth_busy.get()
            on:input=move |ev| {
                auth.password_draft.set(event_target_value(&ev));
                auth.registration_notice.set(None);
            }
        />
    }
}

fn auth_submit_button(
    submit_disabled: Signal<bool>,
    button_label: Signal<&'static str>,
) -> impl IntoView {
    view! {
        <div class="auth-form__actions">
            <button
                type="submit"
                class="auth-form__submit"
                prop:disabled=move || submit_disabled.get()
            >
                {move || button_label.get()}
            </button>
        </div>
    }
}

fn sign_in_button_label(signing_in: bool) -> &'static str {
    if signing_in {
        "Signing in..."
    } else {
        "Sign in"
    }
}

fn sign_up_button_label(signing_up: bool) -> &'static str {
    if signing_up {
        "Creating account..."
    } else {
        "Create account"
    }
}
