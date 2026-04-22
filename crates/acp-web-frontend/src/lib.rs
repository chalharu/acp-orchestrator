//! ACP Web frontend – Leptos CSR, compiled to WebAssembly.

mod application;
mod browser;
mod components;
mod domain;
mod infrastructure;
mod presentation;
mod session;
mod slash;

use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::{
    domain::routing::{AppRoute, current_route},
    presentation::{AccountsPage, RegisterPage, SignInPage},
    session::{HomePage, SessionView},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AppViewKind {
    Home,
    Register,
    SignIn,
    Accounts,
    Session,
    NotFound,
}

fn app_view_kind(route: &AppRoute) -> AppViewKind {
    match route {
        AppRoute::Home => AppViewKind::Home,
        AppRoute::Register => AppViewKind::Register,
        AppRoute::SignIn => AppViewKind::SignIn,
        AppRoute::Accounts => AppViewKind::Accounts,
        AppRoute::Session(_) => AppViewKind::Session,
        AppRoute::NotFound => AppViewKind::NotFound,
    }
}

fn not_found_view() -> impl IntoView {
    view! {
        <main class="app-shell">
            <nav class="shell-nav">
                <a href="/app/">"New chat"</a>
            </nav>
            <section class="panel empty-state">
                <p class="muted">"Page not found."</p>
            </section>
        </main>
    }
}

/// Mount the Leptos app into `<div id="app-root">`.
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to(
        web_sys::window()
            .expect("window must exist")
            .document()
            .expect("document must exist")
            .get_element_by_id("app-root")
            .expect("app-root element must exist in shell")
            .dyn_into::<web_sys::HtmlElement>()
            .expect("app-root element must be an HtmlElement"),
        App,
    )
    .forget();
}

#[component]
fn App() -> impl IntoView {
    view! {
        {move || {
            let route = current_route();
            match app_view_kind(&route) {
                AppViewKind::Home => view! { <HomePage /> }.into_any(),
                AppViewKind::Register => view! { <RegisterPage /> }.into_any(),
                AppViewKind::SignIn => view! { <SignInPage /> }.into_any(),
                AppViewKind::Accounts => view! { <AccountsPage /> }.into_any(),
                AppViewKind::Session => match route {
                    AppRoute::Session(session_id) => view! { <SessionView session_id=session_id /> }.into_any(),
                    _ => unreachable!("app_view_kind must match the route variant"),
                },
                AppViewKind::NotFound => not_found_view().into_any(),
            }
        }}
    }
}

#[cfg(test)]
mod tests {
    use leptos::prelude::*;

    use super::*;

    #[test]
    fn app_view_kind_matches_every_route_variant() {
        assert_eq!(app_view_kind(&AppRoute::Home), AppViewKind::Home);
        assert_eq!(app_view_kind(&AppRoute::Register), AppViewKind::Register);
        assert_eq!(app_view_kind(&AppRoute::SignIn), AppViewKind::SignIn);
        assert_eq!(app_view_kind(&AppRoute::Accounts), AppViewKind::Accounts);
        assert_eq!(
            app_view_kind(&AppRoute::Session("s1".to_string())),
            AppViewKind::Session
        );
        assert_eq!(app_view_kind(&AppRoute::NotFound), AppViewKind::NotFound);
    }

    #[test]
    fn not_found_view_and_app_build_without_panicking_on_host() {
        let owner = Owner::new();
        owner.with(|| {
            let _ = not_found_view();
            let _ = view! { <App /> };
        });
    }
}
