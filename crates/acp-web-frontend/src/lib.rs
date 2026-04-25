//! ACP Web frontend – Leptos CSR, compiled to WebAssembly.

mod application;
mod browser;
mod components;
mod infrastructure;
mod presentation;
mod routing;
mod session_activity;
mod session_lifecycle;
mod session_page;
mod session_page_actions;
mod session_page_bootstrap;
mod session_page_callbacks;
mod session_page_composer_signals;
mod session_page_dock;
mod session_page_entries;
mod session_page_home;
mod session_page_layout;
mod session_page_main;
mod session_page_main_signals;
mod session_page_shell_signals;
mod session_page_sidebar;
mod session_page_sidebar_header;
mod session_page_sidebar_item;
mod session_page_sidebar_list;
mod session_page_sidebar_nav;
mod session_page_sidebar_status;
mod session_page_sidebar_styles;
mod session_page_signals;
mod session_page_topbar;
mod session_page_transcript;
mod session_state;
mod slash;
mod transcript_view;

use leptos::prelude::*;
#[cfg(target_family = "wasm")]
use wasm_bindgen::JsCast;

use crate::presentation::{AccountsPage, RegisterPage, SignInPage, WorkspacesPage};
use crate::routing::{AppRoute, current_route};
use crate::session_page::{HomePage, SessionView};

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

fn workspaces_route_view() -> AnyView {
    view! { <WorkspacesPage /> }.into_any()
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
        AppRoute::Workspaces => workspaces_route_view(),
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
            let _ = dispatch_route(AppRoute::Workspaces);
            let _ = dispatch_route(AppRoute::Session("s1".to_string()));
            let _ = dispatch_route(AppRoute::NotFound);
        });
    }
}
