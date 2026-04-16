//! ACP Web frontend – Leptos CSR, compiled to WebAssembly.
//!
//! Slice 1 minimal chat flow:
//! - `/app/`              – home: compose first prompt, creates session on submit
//! - `/app/sessions/{id}` – live session: transcript, SSE updates, composer
//!
//! Auth: same-origin cookie (`acp_session`).  
//! CSRF: `acp_csrf` cookie + `x-csrf-token` request header (bootstrapped by backend via
//! `<meta name="acp-csrf-token">` in the shell document).

mod api;
mod components;

use leptos::prelude::*;
use wasm_bindgen::JsCast;

use components::{Composer, ErrorBanner, Header, PendingPermissions, Transcript};

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Root application component
// ---------------------------------------------------------------------------

#[component]
fn App() -> impl IntoView {
    view! {
        {move || match current_route() {
            AppRoute::Home => view! { <HomePage /> }.into_any(),
            AppRoute::Session(session_id) => view! { <SessionView session_id=session_id /> }.into_any(),
            AppRoute::NotFound => {
                view! {
                    <main class="app-shell">
                        <Header
                            backend_origin=window_origin()
                            connection_status=Signal::derive(|| "ready".to_string())
                            session_status=Signal::derive(|| "unknown".to_string())
                            route_summary="Unknown ACP Web route.".to_string()
                        />
                        <p class="top-link"><a href="/app/">"Start a fresh chat"</a></p>
                        <section class="panel">
                            <p class="muted">"Page not found."</p>
                        </section>
                    </main>
                }
                    .into_any()
            }
        }}
    }
}

// ---------------------------------------------------------------------------
// Home page  –  /app/
// ---------------------------------------------------------------------------

/// Landing page. Shows an empty transcript and a composer.
/// On first prompt submit it creates a new session then navigates to
/// `/app/sessions/{id}`.
#[component]
fn HomePage() -> impl IntoView {
    let error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);

    let on_submit = move |prompt: String| {
        busy.set(true);
        error.set(None);

        leptos::task::spawn_local(async move {
            match api::create_session_and_send(&prompt).await {
                Ok(session_id) => {
                    if let Err(message) = navigate_to(&format!("/app/sessions/{session_id}")) {
                        error.set(Some(message));
                        busy.set(false);
                    }
                }
                Err(msg) => {
                    error.set(Some(msg));
                    busy.set(false);
                }
            }
        });
    };

    view! {
        <main class="app-shell">
            <Header
                backend_origin=window_origin()
                connection_status=Signal::derive(|| "ready".to_string())
                session_status=Signal::derive(|| "new".to_string())
                route_summary="Send the first prompt to create a session and move to /app/sessions/{id}."
            />
            <p class="top-link"><a href="/app/">"Start a fresh chat"</a></p>
            <ErrorBanner message=error />
            <Transcript entries=Signal::derive(Vec::new) />
            <Composer
                busy=Signal::derive(move || busy.get())
                status_text=Signal::derive(|| "Ready for your first prompt.".to_string())
                on_submit=Callback::new(on_submit)
            />
        </main>
    }
}

// ---------------------------------------------------------------------------
// Session view (inner, keyed on session_id)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct TranscriptEntry {
    pub id: String,
    pub role: EntryRole,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum EntryRole {
    User,
    Assistant,
    Status,
}

impl EntryRole {
    pub fn css_class(&self) -> &'static str {
        match self {
            EntryRole::User => "transcript-entry--user",
            EntryRole::Assistant => "transcript-entry--assistant",
            EntryRole::Status => "transcript-entry--status",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            EntryRole::User => "user",
            EntryRole::Assistant => "assistant",
            EntryRole::Status => "status",
        }
    }
}

#[component]
fn SessionView(session_id: String) -> impl IntoView {
    let id = session_id.clone();
    let entries: RwSignal<Vec<TranscriptEntry>> = RwSignal::new(Vec::new());
    let pending_permissions: RwSignal<Vec<(String, String)>> = RwSignal::new(Vec::new());
    let error = RwSignal::new(None::<String>);
    let connection_status = RwSignal::new("connecting".to_string());
    let session_status = RwSignal::new("loading".to_string());
    let busy = RwSignal::new(false);

    // Bootstrap: load history + open SSE stream.
    let sid = id.clone();
    leptos::task::spawn_local({
        let sid = sid.clone();
        async move {
            match api::load_session(&sid).await {
                Ok(session) => {
                    entries.set(session.entries);
                    pending_permissions.set(session.pending_permissions);
                    session_status.set(session.session_status);
                }
                Err(e) => {
                    error.set(Some(e));
                    connection_status.set("error".to_string());
                    return;
                }
            }

            api::subscribe_sse(
                &sid,
                entries,
                pending_permissions,
                connection_status,
                session_status,
                error,
            )
            .await;
        }
    });

    let sid_for_submit = id.clone();
    let on_submit = move |prompt: String| {
        let sid = sid_for_submit.clone();
        busy.set(true);
        error.set(None);
        leptos::task::spawn_local(async move {
            if let Err(msg) = api::send_message(&sid, &prompt).await {
                error.set(Some(msg));
            }
            busy.set(false);
        });
    };

    let composer_busy = Signal::derive(move || busy.get() || session_status.get() == "closed");
    let composer_status = Signal::derive(move || {
        if busy.get() {
            "Sending...".to_string()
        } else if session_status.get() == "closed" {
            "Session closed.".to_string()
        } else {
            format!("Session {}", session_status.get())
        }
    });

    view! {
        <main class="app-shell">
            <Header
                backend_origin=window_origin()
                connection_status=Signal::derive(move || connection_status.get())
                session_status=Signal::derive(move || session_status.get())
                route_summary=format!("Session: {}", id)
            />
            <p class="top-link"><a href="/app/">"Start a fresh chat"</a></p>
            <ErrorBanner message=error />
            <Transcript entries=Signal::derive(move || entries.get()) />
            <PendingPermissions items=Signal::derive(move || pending_permissions.get()) />
            <Composer
                busy=composer_busy
                status_text=composer_status
                on_submit=Callback::new(on_submit)
            />
        </main>
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn window_origin() -> String {
    web_sys::window()
        .and_then(|w| w.location().origin().ok())
        .unwrap_or_default()
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum AppRoute {
    Home,
    Session(String),
    NotFound,
}

fn current_route() -> AppRoute {
    let Some(pathname) = web_sys::window().and_then(|window| window.location().pathname().ok())
    else {
        return AppRoute::NotFound;
    };

    if pathname == "/app" || pathname == "/app/" {
        return AppRoute::Home;
    }

    pathname
        .strip_prefix("/app/sessions/")
        .filter(|session_id| !session_id.is_empty())
        .map(|session_id| AppRoute::Session(session_id.to_string()))
        .unwrap_or(AppRoute::NotFound)
}

fn navigate_to(path: &str) -> Result<(), String> {
    web_sys::window()
        .ok_or_else(|| "window not available".to_string())?
        .location()
        .set_href(path)
        .map_err(|error| format!("Failed to navigate to {path}: {error:?}"))
}
