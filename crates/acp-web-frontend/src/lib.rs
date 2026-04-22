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
#[cfg(target_family = "wasm")]
use wasm_bindgen::JsCast;

use crate::domain::routing::{AppRoute, current_route};
use crate::presentation::{AccountsPage, RegisterPage, SignInPage};
use crate::session::{HomePage, SessionView};

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

fn home_route_view() -> AnyView {
    view! { <HomePage /> }.into_any()
}

fn register_route_view() -> AnyView {
    view! { <RegisterPage /> }.into_any()
}

fn sign_in_route_view() -> AnyView {
    view! { <SignInPage /> }.into_any()
}

fn accounts_route_view() -> AnyView {
    view! { <AccountsPage /> }.into_any()
}

fn session_route_view(session_id: String) -> AnyView {
    view! { <SessionView session_id=session_id /> }.into_any()
}

fn dispatch_route(route: AppRoute) -> AnyView {
    match route {
        AppRoute::Home => home_route_view(),
        AppRoute::Register => register_route_view(),
        AppRoute::SignIn => sign_in_route_view(),
        AppRoute::Accounts => accounts_route_view(),
        AppRoute::Session(session_id) => session_route_view(session_id),
        AppRoute::NotFound => not_found_view().into_any(),
    }
}

/// Mount the Leptos app into `<div id="app-root">`.
#[cfg(target_family = "wasm")]
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

#[cfg(target_family = "wasm")]
#[component]
fn App() -> impl IntoView {
    view! {
        {move || dispatch_route(current_route())}
    }
}

#[cfg(not(target_family = "wasm"))]
#[component]
fn App() -> impl IntoView {
    dispatch_route(current_route())
}

#[cfg(test)]
mod tests {
    use leptos::prelude::*;

    use super::*;

    #[test]
    fn not_found_view_and_app_build_without_panicking_on_host() {
        let owner = Owner::new();
        owner.with(|| {
            let _ = not_found_view();
            let _ = view! { <App /> };
        });
    }

    #[test]
    fn dispatch_route_builds_all_routes_on_host() {
        let owner = Owner::new();
        owner.with(|| {
            let _ = dispatch_route(AppRoute::Home);
            let _ = dispatch_route(AppRoute::Register);
            let _ = dispatch_route(AppRoute::SignIn);
            let _ = dispatch_route(AppRoute::Accounts);
            let _ = dispatch_route(AppRoute::Session("s1".to_string()));
            let _ = dispatch_route(AppRoute::NotFound);
        });
    }
}
