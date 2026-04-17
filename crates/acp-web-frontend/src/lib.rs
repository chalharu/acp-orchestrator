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

use acp_contracts::PermissionDecision;
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
                            route_summary="This route is not available in the current web shell."
                                .to_string()
                        />
                        <p class="top-link"><a href="/app/">"Start a new chat"</a></p>
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
    let draft = RwSignal::new(String::new());

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
                route_summary="Send the first prompt here to create a session and move into the conversation view."
            />
            <p class="top-link"><a href="/app/">"Start a new chat"</a></p>
            <ErrorBanner message=error />
            <Transcript entries=Signal::derive(Vec::new) />
            <Composer
                busy=Signal::derive(move || busy.get())
                status_text=Signal::derive(|| "Ready for your first prompt.".to_string())
                draft=draft
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

#[derive(Clone, Copy)]
struct SessionSignals {
    entries: RwSignal<Vec<TranscriptEntry>>,
    pending_permissions: RwSignal<Vec<(String, String)>>,
    error: RwSignal<Option<String>>,
    connection_status: RwSignal<String>,
    session_status: RwSignal<String>,
    busy: RwSignal<bool>,
    pending_action_busy: RwSignal<bool>,
    draft: RwSignal<String>,
}

struct SessionViewCallbacks {
    submit: Callback<String>,
    approve: Callback<String>,
    deny: Callback<String>,
    cancel: Callback<()>,
}

#[component]
fn SessionView(session_id: String) -> impl IntoView {
    let signals = session_signals();

    spawn_session_bootstrap(
        session_id.clone(),
        signals.entries,
        signals.pending_permissions,
        signals.connection_status,
        signals.session_status,
        signals.error,
    );

    let composer_busy = session_composer_busy_signal(
        signals.busy,
        signals.session_status,
        signals.pending_permissions,
    );
    let composer_status = session_composer_status_signal(
        signals.busy,
        signals.session_status,
        signals.pending_permissions,
    );
    let on_submit = session_submit_callback(
        session_id.clone(),
        signals.busy,
        signals.error,
        signals.draft,
    );
    let (on_approve, on_deny, on_cancel) = session_permission_callbacks(
        session_id.clone(),
        signals.pending_permissions,
        signals.pending_action_busy,
        signals.error,
    );
    let callbacks = SessionViewCallbacks {
        submit: on_submit,
        approve: on_approve,
        deny: on_deny,
        cancel: on_cancel,
    };

    session_view_content(
        session_id,
        signals,
        composer_busy,
        composer_status,
        callbacks,
    )
}

fn session_signals() -> SessionSignals {
    SessionSignals {
        entries: RwSignal::new(Vec::new()),
        pending_permissions: RwSignal::new(Vec::new()),
        error: RwSignal::new(None::<String>),
        connection_status: RwSignal::new("connecting".to_string()),
        session_status: RwSignal::new("loading".to_string()),
        busy: RwSignal::new(false),
        pending_action_busy: RwSignal::new(false),
        draft: RwSignal::new(String::new()),
    }
}

fn session_view_content(
    session_id: String,
    signals: SessionSignals,
    composer_busy: Signal<bool>,
    composer_status: Signal<String>,
    callbacks: SessionViewCallbacks,
) -> impl IntoView {
    let entries = signals.entries;
    let pending_permissions = signals.pending_permissions;
    let pending_action_busy = signals.pending_action_busy;
    let error = signals.error;
    let connection_status = signals.connection_status;
    let session_status = signals.session_status;
    let draft = signals.draft;
    let SessionViewCallbacks {
        submit: on_submit,
        approve: on_approve,
        deny: on_deny,
        cancel: on_cancel,
    } = callbacks;

    view! {
        <main class="app-shell">
            <Header
                backend_origin=window_origin()
                connection_status=Signal::derive(move || connection_status.get())
                session_status=Signal::derive(move || session_status.get())
                route_summary=format!("You are viewing session {session_id}.")
            />
            <p class="top-link"><a href="/app/">"Start a new chat"</a></p>
            <ErrorBanner message=error />
            <Transcript entries=Signal::derive(move || entries.get()) />
            <PendingPermissions
                items=Signal::derive(move || pending_permissions.get())
                busy=Signal::derive(move || pending_action_busy.get())
                on_approve=on_approve
                on_deny=on_deny
                on_cancel=on_cancel
            />
            <Composer
                busy=composer_busy
                status_text=composer_status
                draft=draft
                on_submit=on_submit
            />
        </main>
    }
}

fn session_permission_callbacks(
    session_id: String,
    pending_permissions: RwSignal<Vec<(String, String)>>,
    pending_action_busy: RwSignal<bool>,
    error: RwSignal<Option<String>>,
) -> (Callback<String>, Callback<String>, Callback<()>) {
    (
        permission_resolution_callback(
            session_id.clone(),
            PermissionDecision::Approve,
            pending_permissions,
            pending_action_busy,
            error,
        ),
        permission_resolution_callback(
            session_id.clone(),
            PermissionDecision::Deny,
            pending_permissions,
            pending_action_busy,
            error,
        ),
        cancel_turn_callback(session_id, pending_permissions, pending_action_busy, error),
    )
}

fn spawn_session_bootstrap(
    session_id: String,
    entries: RwSignal<Vec<TranscriptEntry>>,
    pending_permissions: RwSignal<Vec<(String, String)>>,
    connection_status: RwSignal<String>,
    session_status: RwSignal<String>,
    error: RwSignal<Option<String>>,
) {
    leptos::task::spawn_local(async move {
        match api::load_session(&session_id).await {
            Ok(session) => {
                entries.set(session.entries);
                pending_permissions.set(session.pending_permissions);
                session_status.set(session.session_status);
            }
            Err(api::SessionLoadError::ResumeUnavailable(message)) => {
                error.set(Some(message));
                connection_status.set("unavailable".to_string());
                session_status.set("unavailable".to_string());
                return;
            }
            Err(api::SessionLoadError::Other(message)) => {
                error.set(Some(message));
                connection_status.set("error".to_string());
                session_status.set("error".to_string());
                return;
            }
        }

        api::subscribe_sse(
            &session_id,
            entries,
            pending_permissions,
            connection_status,
            session_status,
            error,
        )
        .await;
    });
}

fn session_submit_callback(
    session_id: String,
    busy: RwSignal<bool>,
    error: RwSignal<Option<String>>,
    draft: RwSignal<String>,
) -> Callback<String> {
    Callback::new(move |prompt: String| {
        let session_id = session_id.clone();
        busy.set(true);
        error.set(None);
        leptos::task::spawn_local(async move {
            match api::send_message(&session_id, &prompt).await {
                Ok(()) => draft.set(String::new()),
                Err(message) => error.set(Some(message)),
            }
            busy.set(false);
        });
    })
}

fn permission_resolution_callback(
    session_id: String,
    decision: PermissionDecision,
    pending_permissions: RwSignal<Vec<(String, String)>>,
    pending_action_busy: RwSignal<bool>,
    error: RwSignal<Option<String>>,
) -> Callback<String> {
    Callback::new(move |request_id: String| {
        let session_id = session_id.clone();
        let request_id_for_state = request_id.clone();
        let decision = decision.clone();
        pending_action_busy.set(true);
        error.set(None);
        leptos::task::spawn_local(async move {
            match api::resolve_permission(&session_id, &request_id, decision).await {
                Ok(_) => pending_permissions.update(|current_permissions| {
                    current_permissions.retain(|(current_request_id, _)| {
                        current_request_id != &request_id_for_state
                    });
                }),
                Err(message) => error.set(Some(message)),
            }
            pending_action_busy.set(false);
        });
    })
}

fn cancel_turn_callback(
    session_id: String,
    pending_permissions: RwSignal<Vec<(String, String)>>,
    pending_action_busy: RwSignal<bool>,
    error: RwSignal<Option<String>>,
) -> Callback<()> {
    Callback::new(move |()| {
        let session_id = session_id.clone();
        pending_action_busy.set(true);
        error.set(None);
        leptos::task::spawn_local(async move {
            match api::cancel_turn(&session_id).await {
                Ok(cancelled) if cancelled.cancelled => pending_permissions.set(Vec::new()),
                Ok(_) => error.set(Some("No running turn is active.".to_string())),
                Err(message) => error.set(Some(message)),
            }
            pending_action_busy.set(false);
        });
    })
}

fn session_composer_busy_signal(
    busy: RwSignal<bool>,
    session_status: RwSignal<String>,
    pending_permissions: RwSignal<Vec<(String, String)>>,
) -> Signal<bool> {
    Signal::derive(move || {
        let session_status = session_status.get();
        let has_pending_permissions = !pending_permissions.get().is_empty();
        session_composer_disabled(busy.get(), &session_status, has_pending_permissions)
    })
}

fn session_composer_status_signal(
    busy: RwSignal<bool>,
    session_status: RwSignal<String>,
    pending_permissions: RwSignal<Vec<(String, String)>>,
) -> Signal<String> {
    Signal::derive(move || {
        let session_status = session_status.get();
        let has_pending_permissions = !pending_permissions.get().is_empty();
        session_composer_status_message(busy.get(), &session_status, has_pending_permissions)
    })
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

fn session_composer_disabled(
    busy: bool,
    session_status: &str,
    has_pending_permissions: bool,
) -> bool {
    busy || has_pending_permissions || session_status != "active"
}

fn session_composer_status_message(
    busy: bool,
    session_status: &str,
    has_pending_permissions: bool,
) -> String {
    if busy {
        "Sending...".to_string()
    } else if has_pending_permissions {
        "Resolve or cancel the pending permission request before sending another prompt."
            .to_string()
    } else {
        match session_status {
            "active" => "Session active.".to_string(),
            "closed" => "Session closed.".to_string(),
            "loading" => "Loading session...".to_string(),
            "unavailable" | "error" => "Session unavailable. Start a fresh chat.".to_string(),
            status => format!("Session {status}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{session_composer_disabled, session_composer_status_message};

    #[test]
    fn session_composer_is_disabled_while_permissions_are_pending() {
        assert!(session_composer_disabled(false, "active", true));
    }

    #[test]
    fn session_composer_prompts_for_permission_resolution_before_new_messages() {
        assert_eq!(
            session_composer_status_message(false, "active", true),
            "Resolve or cancel the pending permission request before sending another prompt."
        );
    }
}
