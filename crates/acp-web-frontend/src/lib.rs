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
        {move || match current_route() {
            AppRoute::Home => view! { <HomePage /> }.into_any(),
            AppRoute::Register => view! { <RegisterPage /> }.into_any(),
            AppRoute::SignIn => view! { <SignInPage /> }.into_any(),
            AppRoute::Accounts => view! { <AccountsPage /> }.into_any(),
            AppRoute::Session(session_id) => view! { <SessionView session_id=session_id /> }.into_any(),
            AppRoute::NotFound => {
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
                    .into_any()
            }
        }}
    }
}
