use leptos::prelude::*;

use crate::{
    application::auth::{HomeRouteTarget, home_route_target},
    browser::navigate_to,
    components::ErrorBanner,
    infrastructure::api,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthPageKind {
    Register,
    SignIn,
}

#[derive(Clone, Copy)]
pub struct AuthPageState {
    pub username: RwSignal<String>,
    pub password: RwSignal<String>,
    pub error: RwSignal<Option<String>>,
    pub loading: RwSignal<bool>,
    pub submitting: RwSignal<bool>,
    checked: RwSignal<bool>,
}

impl AuthPageState {
    pub fn new() -> Self {
        Self {
            username: RwSignal::new(String::new()),
            password: RwSignal::new(String::new()),
            error: RwSignal::new(None::<String>),
            loading: RwSignal::new(true),
            submitting: RwSignal::new(false),
            checked: RwSignal::new(false),
        }
    }
}

pub fn initialize_auth_page(kind: AuthPageKind, state: AuthPageState) {
    Effect::new(move |_| {
        if state.checked.get() {
            return;
        }

        state.checked.set(true);
        leptos::task::spawn_local(async move {
            match api::auth_status().await {
                Ok(status) => handle_auth_status(kind, home_route_target(&status), state),
                Err(message) => {
                    state.loading.set(false);
                    state.error.set(Some(message));
                }
            }
        });
    });
}

pub fn submit_credentials_handler(
    kind: AuthPageKind,
    state: AuthPageState,
) -> Callback<web_sys::SubmitEvent> {
    Callback::new(move |event: web_sys::SubmitEvent| {
        event.prevent_default();
        if state.submitting.get_untracked() {
            return;
        }

        state.submitting.set(true);
        state.error.set(None);
        let username = state.username.get_untracked();
        let password = state.password.get_untracked();
        leptos::task::spawn_local(async move {
            match submit_credentials(kind, &username, &password).await {
                Ok(()) => {
                    let _ = navigate_to("/app/");
                }
                Err(message) => {
                    state.submitting.set(false);
                    state.error.set(Some(message));
                }
            }
        });
    })
}

pub fn auth_page_view(
    kind: AuthPageKind,
    state: AuthPageState,
    on_submit: Callback<web_sys::SubmitEvent>,
) -> impl IntoView {
    view! {
        <main class="app-shell app-shell--home">
            <ErrorBanner message=state.error />
            <section class="panel account-panel">
                <h1>{page_title(kind)}</h1>
                <Show
                    when=move || !state.loading.get()
                    fallback=move || view! { <p class="muted">{loading_message(kind)}</p> }
                >
                    <form class="account-form" on:submit=move |event| on_submit.run(event)>
                        <label class="account-form__field">
                            <span>"User name"</span>
                            <input
                                type="text"
                                prop:value=move || state.username.get()
                                on:input=move |event| state.username.set(event_target_value(&event))
                                autocomplete="username"
                            />
                        </label>
                        <label class="account-form__field">
                            <span>"Password"</span>
                            <input
                                type="password"
                                prop:value=move || state.password.get()
                                on:input=move |event| state.password.set(event_target_value(&event))
                                autocomplete=password_autocomplete(kind)
                            />
                        </label>
                        <button
                            type="submit"
                            class="account-form__submit"
                            prop:disabled=move || state.submitting.get()
                        >
                            {move || submit_button_label(kind, state.submitting.get())}
                        </button>
                    </form>
                </Show>
            </section>
        </main>
    }
}

fn handle_auth_status(kind: AuthPageKind, target: HomeRouteTarget, state: AuthPageState) {
    match auth_page_route(kind, target) {
        AuthPageRoute::Ready => state.loading.set(false),
        AuthPageRoute::Redirect(path) => {
            let _ = navigate_to(path);
        }
    }
}

async fn submit_credentials(
    kind: AuthPageKind,
    username: &str,
    password: &str,
) -> Result<(), String> {
    match kind {
        AuthPageKind::Register => api::bootstrap_register(username, password)
            .await
            .map(|_| ()),
        AuthPageKind::SignIn => api::sign_in(username, password).await.map(|_| ()),
    }
}

fn page_title(kind: AuthPageKind) -> &'static str {
    match kind {
        AuthPageKind::Register => "Register bootstrap account",
        AuthPageKind::SignIn => "Sign in",
    }
}

fn loading_message(kind: AuthPageKind) -> &'static str {
    match kind {
        AuthPageKind::Register => "Checking registration status…",
        AuthPageKind::SignIn => "Checking sign-in status…",
    }
}

fn password_autocomplete(kind: AuthPageKind) -> &'static str {
    match kind {
        AuthPageKind::Register => "new-password",
        AuthPageKind::SignIn => "current-password",
    }
}

fn submit_button_label(kind: AuthPageKind, submitting: bool) -> &'static str {
    match (kind, submitting) {
        (AuthPageKind::Register, true) => "Creating account…",
        (AuthPageKind::Register, false) => "Create account",
        (AuthPageKind::SignIn, true) => "Signing in…",
        (AuthPageKind::SignIn, false) => "Sign in",
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AuthPageRoute {
    Ready,
    Redirect(&'static str),
}

fn auth_page_route(kind: AuthPageKind, target: HomeRouteTarget) -> AuthPageRoute {
    match (kind, target) {
        (_, HomeRouteTarget::PrepareSession) => AuthPageRoute::Redirect("/app/"),
        (AuthPageKind::Register, HomeRouteTarget::Register) => AuthPageRoute::Ready,
        (AuthPageKind::Register, HomeRouteTarget::SignIn) => {
            AuthPageRoute::Redirect("/app/sign-in/")
        }
        (AuthPageKind::SignIn, HomeRouteTarget::Register) => {
            AuthPageRoute::Redirect("/app/register/")
        }
        (AuthPageKind::SignIn, HomeRouteTarget::SignIn) => AuthPageRoute::Ready,
    }
}

#[cfg(test)]
mod tests {
    use leptos::prelude::*;

    use super::*;

    #[test]
    fn auth_page_route_redirects_to_the_expected_destination() {
        assert_eq!(
            auth_page_route(AuthPageKind::Register, HomeRouteTarget::PrepareSession),
            AuthPageRoute::Redirect("/app/")
        );
        assert_eq!(
            auth_page_route(AuthPageKind::Register, HomeRouteTarget::SignIn),
            AuthPageRoute::Redirect("/app/sign-in/")
        );
        assert_eq!(
            auth_page_route(AuthPageKind::SignIn, HomeRouteTarget::Register),
            AuthPageRoute::Redirect("/app/register/")
        );
    }

    #[test]
    fn auth_page_route_keeps_matching_pages_ready() {
        assert_eq!(
            auth_page_route(AuthPageKind::Register, HomeRouteTarget::Register),
            AuthPageRoute::Ready
        );
        assert_eq!(
            auth_page_route(AuthPageKind::SignIn, HomeRouteTarget::SignIn),
            AuthPageRoute::Ready
        );
    }

    #[test]
    fn submit_labels_and_autocomplete_match_each_page() {
        assert_eq!(
            password_autocomplete(AuthPageKind::Register),
            "new-password"
        );
        assert_eq!(
            password_autocomplete(AuthPageKind::SignIn),
            "current-password"
        );
        assert_eq!(
            submit_button_label(AuthPageKind::Register, true),
            "Creating account…"
        );
        assert_eq!(submit_button_label(AuthPageKind::SignIn, false), "Sign in");
    }

    #[test]
    fn page_title_and_loading_message_differ_by_page_kind() {
        assert_eq!(
            page_title(AuthPageKind::Register),
            "Register bootstrap account"
        );
        assert_eq!(page_title(AuthPageKind::SignIn), "Sign in");
        assert_eq!(
            loading_message(AuthPageKind::Register),
            "Checking registration status…"
        );
        assert_eq!(
            loading_message(AuthPageKind::SignIn),
            "Checking sign-in status…"
        );
    }

    #[test]
    fn submit_button_label_covers_remaining_combinations() {
        assert_eq!(
            submit_button_label(AuthPageKind::Register, false),
            "Create account"
        );
        assert_eq!(
            submit_button_label(AuthPageKind::SignIn, true),
            "Signing in…"
        );
    }

    #[test]
    fn auth_page_state_starts_with_blank_inputs_and_loading_enabled() {
        let owner = Owner::new();
        owner.with(|| {
            let state = AuthPageState::new();
            assert!(state.username.get().is_empty());
            assert!(state.password.get().is_empty());
            assert!(state.error.get().is_none());
            assert!(state.loading.get());
            assert!(!state.submitting.get());
        });
    }

    #[test]
    fn handle_auth_status_updates_ready_and_redirect_routes() {
        let owner = Owner::new();
        owner.with(|| {
            let ready_state = AuthPageState::new();
            handle_auth_status(
                AuthPageKind::Register,
                HomeRouteTarget::Register,
                ready_state,
            );
            assert!(!ready_state.loading.get());

            let redirect_state = AuthPageState::new();
            handle_auth_status(
                AuthPageKind::SignIn,
                HomeRouteTarget::PrepareSession,
                redirect_state,
            );
            assert!(redirect_state.loading.get());
            assert!(redirect_state.error.get().is_none());
        });
    }

    #[test]
    fn auth_page_view_builds_register_and_sign_in_forms() {
        let owner = Owner::new();
        owner.with(|| {
            let register = AuthPageState::new();
            register.loading.set(false);
            let sign_in = AuthPageState::new();
            sign_in.loading.set(false);

            let _ = auth_page_view(
                AuthPageKind::Register,
                register,
                Callback::new(|_: web_sys::SubmitEvent| {}),
            );
            let _ = auth_page_view(
                AuthPageKind::SignIn,
                sign_in,
                Callback::new(|_: web_sys::SubmitEvent| {}),
            );
        });
    }

    #[test]
    fn auth_page_view_builds_loading_fallbacks() {
        let owner = Owner::new();
        owner.with(|| {
            let register = AuthPageState::new();
            let sign_in = AuthPageState::new();

            let _ = auth_page_view(
                AuthPageKind::Register,
                register,
                Callback::new(|_: web_sys::SubmitEvent| {}),
            );
            let _ = auth_page_view(
                AuthPageKind::SignIn,
                sign_in,
                Callback::new(|_: web_sys::SubmitEvent| {}),
            );
        });
    }
}
