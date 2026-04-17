//! ACP Web frontend – Leptos CSR, compiled to WebAssembly.
//!
//! Slice 1 minimal chat flow:
//! - `/app/`              – prepares a fresh session, then redirects into chat
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

use components::{Composer, ErrorBanner, PendingPermissions, Transcript};

const PREPARED_SESSION_STORAGE_KEY: &str = "acp-prepared-session-id";

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

// ---------------------------------------------------------------------------
// Home page  –  /app/
// ---------------------------------------------------------------------------

/// Landing page. Prepares a fresh session and immediately redirects to the
/// live chat route so startup hints appear before the first prompt.
#[component]
fn HomePage() -> impl IntoView {
    let error = RwSignal::new(None::<String>);
    let preparing = RwSignal::new(true);
    let started = RwSignal::new(false);

    Effect::new(move |_| {
        if started.get() {
            return;
        }

        started.set(true);
        error.set(None);
        spawn_home_redirect(error, preparing);
    });

    view! {
        <main class="app-shell app-shell--home">
            <ErrorBanner message=error />
            <section class="panel empty-state">
                <p class="muted">
                    {move || if preparing.get() {
                        "Preparing chat..."
                    } else {
                        "Unable to prepare a new chat."
                    }}
                </p>
            </section>
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TurnState {
    Idle,
    Submitting,
    AwaitingReply,
    AwaitingPermission,
    Cancelling,
}

#[derive(Clone, Copy)]
struct SessionSignals {
    entries: RwSignal<Vec<TranscriptEntry>>,
    pending_permissions: RwSignal<Vec<(String, String)>>,
    error: RwSignal<Option<String>>,
    connection_status: RwSignal<String>,
    session_status: RwSignal<String>,
    turn_state: RwSignal<TurnState>,
    pending_action_busy: RwSignal<bool>,
    draft: RwSignal<String>,
}

struct SessionViewCallbacks {
    submit: Callback<String>,
    approve: Callback<String>,
    deny: Callback<String>,
    cancel: Callback<()>,
}

#[derive(Clone, Copy)]
struct SessionComposerSignals {
    disabled: Signal<bool>,
    status: Signal<String>,
    cancel_visible: Signal<bool>,
    cancel_busy: Signal<bool>,
}

#[component]
fn SessionView(session_id: String) -> impl IntoView {
    let signals = session_signals();

    spawn_session_bootstrap(session_id.clone(), signals);

    let composer = SessionComposerSignals {
        disabled: session_composer_disabled_signal(signals.turn_state, signals.session_status),
        status: session_composer_status_signal(signals.turn_state, signals.session_status),
        cancel_visible: session_composer_cancel_visible_signal(
            signals.turn_state,
            signals.pending_permissions,
        ),
        cancel_busy: session_composer_cancel_busy_signal(
            signals.turn_state,
            signals.pending_action_busy,
        ),
    };
    let on_submit = session_submit_callback(
        session_id.clone(),
        signals.turn_state,
        signals.error,
        signals.draft,
    );
    let (on_approve, on_deny, on_cancel) = session_permission_callbacks(
        session_id.clone(),
        signals.turn_state,
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

    session_view_content(signals, composer, callbacks)
}

fn session_signals() -> SessionSignals {
    SessionSignals {
        entries: RwSignal::new(Vec::new()),
        pending_permissions: RwSignal::new(Vec::new()),
        error: RwSignal::new(None::<String>),
        connection_status: RwSignal::new("connecting".to_string()),
        session_status: RwSignal::new("loading".to_string()),
        turn_state: RwSignal::new(TurnState::Idle),
        pending_action_busy: RwSignal::new(false),
        draft: RwSignal::new(String::new()),
    }
}

fn session_view_content(
    signals: SessionSignals,
    composer: SessionComposerSignals,
    callbacks: SessionViewCallbacks,
) -> impl IntoView {
    let entries = signals.entries;
    let pending_permissions = signals.pending_permissions;
    let pending_action_busy = signals.pending_action_busy;
    let error = signals.error;
    let draft = signals.draft;
    let SessionViewCallbacks {
        submit: on_submit,
        approve: on_approve,
        deny: on_deny,
        cancel: on_cancel,
    } = callbacks;
    let on_cancel_for_permissions = on_cancel;
    let on_cancel_for_composer = on_cancel;

    view! {
        <main class="app-shell app-shell--session">
            <SessionTopBar message=error />
            <div class="chat-body">
                <Transcript entries=Signal::derive(move || entries.get()) />
            </div>
            <SessionDock
                pending_permissions=Signal::derive(move || pending_permissions.get())
                pending_action_busy=Signal::derive(move || pending_action_busy.get())
                on_approve=on_approve
                on_deny=on_deny
                on_cancel=on_cancel_for_permissions
                composer_disabled=composer.disabled
                composer_status=composer.status
                draft=draft
                on_submit=on_submit
                composer_cancel_visible=composer.cancel_visible
                composer_cancel_busy=composer.cancel_busy
                composer_cancel=on_cancel_for_composer
            />
        </main>
    }
}

#[component]
fn SessionTopBar(message: RwSignal<Option<String>>) -> impl IntoView {
    view! {
        <div class="chat-topbar">
            <nav class="shell-nav">
                <a href="/app/">"New chat"</a>
            </nav>
            <ErrorBanner message=message />
        </div>
    }
}

#[component]
fn SessionDock(
    #[prop(into)] pending_permissions: Signal<Vec<(String, String)>>,
    #[prop(into)] pending_action_busy: Signal<bool>,
    on_approve: Callback<String>,
    on_deny: Callback<String>,
    on_cancel: Callback<()>,
    #[prop(into)] composer_disabled: Signal<bool>,
    #[prop(into)] composer_status: Signal<String>,
    draft: RwSignal<String>,
    on_submit: Callback<String>,
    #[prop(into)] composer_cancel_visible: Signal<bool>,
    #[prop(into)] composer_cancel_busy: Signal<bool>,
    composer_cancel: Callback<()>,
) -> impl IntoView {
    view! {
        <div class="chat-dock">
            <PendingPermissions
                items=pending_permissions
                busy=pending_action_busy
                on_approve=on_approve
                on_deny=on_deny
                on_cancel=on_cancel
            />
            <Composer
                disabled=composer_disabled
                status_text=composer_status
                draft=draft
                on_submit=on_submit
                show_cancel=composer_cancel_visible
                cancel_disabled=composer_cancel_busy
                on_cancel=composer_cancel
            />
        </div>
    }
}

fn session_permission_callbacks(
    session_id: String,
    turn_state: RwSignal<TurnState>,
    pending_permissions: RwSignal<Vec<(String, String)>>,
    pending_action_busy: RwSignal<bool>,
    error: RwSignal<Option<String>>,
) -> (Callback<String>, Callback<String>, Callback<()>) {
    (
        permission_resolution_callback(
            session_id.clone(),
            PermissionDecision::Approve,
            turn_state,
            pending_permissions,
            pending_action_busy,
            error,
        ),
        permission_resolution_callback(
            session_id.clone(),
            PermissionDecision::Deny,
            turn_state,
            pending_permissions,
            pending_action_busy,
            error,
        ),
        cancel_turn_callback(
            session_id,
            turn_state,
            pending_permissions,
            pending_action_busy,
            error,
        ),
    )
}

fn spawn_session_bootstrap(session_id: String, signals: SessionSignals) {
    leptos::task::spawn_local(async move {
        match api::load_session(&session_id).await {
            Ok(session) => apply_loaded_session(session, signals),
            Err(api::SessionLoadError::ResumeUnavailable(message)) => {
                record_session_bootstrap_failure(message, "unavailable", signals);
                return;
            }
            Err(api::SessionLoadError::Other(message)) => {
                record_session_bootstrap_failure(message, "error", signals);
                return;
            }
        }

        api::subscribe_sse(
            &session_id,
            signals.entries,
            signals.pending_permissions,
            signals.connection_status,
            signals.session_status,
            signals.turn_state,
            signals.error,
        )
        .await;
    });
}

fn session_submit_callback(
    session_id: String,
    turn_state: RwSignal<TurnState>,
    error: RwSignal<Option<String>>,
    draft: RwSignal<String>,
) -> Callback<String> {
    Callback::new(move |prompt: String| {
        let session_id = session_id.clone();
        turn_state.set(TurnState::Submitting);
        error.set(None);
        leptos::task::spawn_local(async move {
            match api::send_message(&session_id, &prompt).await {
                Ok(()) => {
                    clear_prepared_session_id();
                    draft.set(String::new());
                    turn_state.set(TurnState::AwaitingReply);
                }
                Err(message) => {
                    error.set(Some(message));
                    turn_state.set(TurnState::Idle);
                }
            }
        });
    })
}

fn spawn_home_redirect(error: RwSignal<Option<String>>, preparing: RwSignal<bool>) {
    leptos::task::spawn_local(async move {
        match resolve_home_session_id().await {
            Ok(session_id) => {
                if let Err(message) = navigate_to(&format!("/app/sessions/{session_id}")) {
                    clear_prepared_session_id();
                    error.set(Some(message));
                    preparing.set(false);
                }
            }
            Err(message) => {
                error.set(Some(message));
                preparing.set(false);
            }
        }
    });
}

async fn resolve_home_session_id() -> Result<String, String> {
    if let Some(session_id) = prepared_session_id() {
        Ok(session_id)
    } else {
        let session_id = api::create_session().await?;
        store_prepared_session_id(&session_id);
        Ok(session_id)
    }
}

fn apply_loaded_session(session: api::SessionBootstrap, signals: SessionSignals) {
    let turn_state_for_session = turn_state_for_snapshot(&session.pending_permissions);
    let should_clear_prepared_session = session.session_status == "closed"
        || session
            .entries
            .iter()
            .any(|entry| matches!(entry.role, EntryRole::User));

    signals.entries.set(session.entries);
    signals.pending_permissions.set(session.pending_permissions);
    signals.session_status.set(session.session_status);
    signals.turn_state.set(turn_state_for_session);
    if should_clear_prepared_session {
        clear_prepared_session_id();
    }
}

fn record_session_bootstrap_failure(
    message: String,
    connection_label: &str,
    signals: SessionSignals,
) {
    clear_prepared_session_id();
    signals.error.set(Some(message));
    signals.connection_status.set(connection_label.to_string());
    signals.session_status.set(connection_label.to_string());
    signals.turn_state.set(TurnState::Idle);
}

fn permission_resolution_callback(
    session_id: String,
    decision: PermissionDecision,
    turn_state: RwSignal<TurnState>,
    pending_permissions: RwSignal<Vec<(String, String)>>,
    pending_action_busy: RwSignal<bool>,
    error: RwSignal<Option<String>>,
) -> Callback<String> {
    Callback::new(move |request_id: String| {
        let session_id = session_id.clone();
        let request_id_for_state = request_id.clone();
        let decision = decision.clone();
        let request_decision = decision.clone();
        pending_action_busy.set(true);
        error.set(None);
        leptos::task::spawn_local(async move {
            match api::resolve_permission(&session_id, &request_id, request_decision).await {
                Ok(_) => {
                    pending_permissions.update(|current_permissions| {
                        current_permissions.retain(|(current_request_id, _)| {
                            current_request_id != &request_id_for_state
                        });
                    });
                    turn_state.set(match decision {
                        PermissionDecision::Approve => TurnState::AwaitingReply,
                        PermissionDecision::Deny => TurnState::Idle,
                    });
                }
                Err(message) => {
                    error.set(Some(message));
                }
            }
            pending_action_busy.set(false);
        });
    })
}

fn cancel_turn_callback(
    session_id: String,
    turn_state: RwSignal<TurnState>,
    pending_permissions: RwSignal<Vec<(String, String)>>,
    pending_action_busy: RwSignal<bool>,
    error: RwSignal<Option<String>>,
) -> Callback<()> {
    Callback::new(move |()| {
        let session_id = session_id.clone();
        let previous_turn_state = turn_state.get_untracked();
        pending_action_busy.set(true);
        turn_state.set(TurnState::Cancelling);
        error.set(None);
        leptos::task::spawn_local(async move {
            match api::cancel_turn(&session_id).await {
                Ok(cancelled) if cancelled.cancelled => {
                    pending_permissions.set(Vec::new());
                    turn_state.set(TurnState::Idle);
                }
                Ok(_) => {
                    error.set(Some("No running turn is active.".to_string()));
                    if turn_state.get_untracked() == TurnState::Cancelling {
                        turn_state.set(previous_turn_state);
                    }
                }
                Err(message) => {
                    error.set(Some(message));
                    if turn_state.get_untracked() == TurnState::Cancelling {
                        turn_state.set(previous_turn_state);
                    }
                }
            }
            pending_action_busy.set(false);
        });
    })
}

fn session_composer_disabled_signal(
    turn_state: RwSignal<TurnState>,
    session_status: RwSignal<String>,
) -> Signal<bool> {
    Signal::derive(move || session_composer_disabled(&session_status.get(), turn_state.get()))
}

fn session_composer_status_signal(
    turn_state: RwSignal<TurnState>,
    session_status: RwSignal<String>,
) -> Signal<String> {
    Signal::derive(move || session_composer_status_message(&session_status.get(), turn_state.get()))
}

fn session_composer_cancel_visible_signal(
    turn_state: RwSignal<TurnState>,
    pending_permissions: RwSignal<Vec<(String, String)>>,
) -> Signal<bool> {
    Signal::derive(move || {
        session_composer_cancel_visible(turn_state.get(), !pending_permissions.get().is_empty())
    })
}

fn session_composer_cancel_busy_signal(
    turn_state: RwSignal<TurnState>,
    pending_action_busy: RwSignal<bool>,
) -> Signal<bool> {
    Signal::derive(move || {
        pending_action_busy.get() || matches!(turn_state.get(), TurnState::Cancelling)
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

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

fn prepared_session_id() -> Option<String> {
    session_storage()
        .and_then(|storage| {
            storage
                .get_item(PREPARED_SESSION_STORAGE_KEY)
                .ok()
                .flatten()
        })
        .filter(|session_id| !session_id.is_empty())
}

fn store_prepared_session_id(session_id: &str) {
    if let Some(storage) = session_storage() {
        let _ = storage.set_item(PREPARED_SESSION_STORAGE_KEY, session_id);
    }
}

fn clear_prepared_session_id() {
    if let Some(storage) = session_storage() {
        let _ = storage.remove_item(PREPARED_SESSION_STORAGE_KEY);
    }
}

fn session_storage() -> Option<web_sys::Storage> {
    web_sys::window().and_then(|window| window.session_storage().ok().flatten())
}

fn session_composer_disabled(session_status: &str, turn_state: TurnState) -> bool {
    session_status != "active" || turn_state != TurnState::Idle
}

fn session_composer_status_message(session_status: &str, turn_state: TurnState) -> String {
    match turn_state {
        TurnState::Submitting | TurnState::AwaitingReply => "Waiting for response...".to_string(),
        TurnState::AwaitingPermission => {
            "Resolve the request below before sending another message.".to_string()
        }
        TurnState::Cancelling => "Cancelling...".to_string(),
        TurnState::Idle => match session_status {
            "active" => String::new(),
            "closed" => "This session is closed.".to_string(),
            "loading" => "Connecting...".to_string(),
            "unavailable" | "error" => "Session unavailable. Start a fresh chat.".to_string(),
            status => format!("Session {status}."),
        },
    }
}

fn session_composer_cancel_visible(turn_state: TurnState, has_pending_permissions: bool) -> bool {
    !has_pending_permissions
        && matches!(turn_state, TurnState::AwaitingReply | TurnState::Cancelling)
}

pub(crate) fn should_apply_snapshot_turn_state(current: TurnState) -> bool {
    matches!(current, TurnState::Idle | TurnState::AwaitingPermission)
}

pub(crate) fn should_release_turn_state_for_assistant_message(current: TurnState) -> bool {
    matches!(current, TurnState::AwaitingReply | TurnState::Cancelling)
}

pub(crate) fn should_release_turn_state_for_status(current: TurnState) -> bool {
    matches!(current, TurnState::AwaitingReply | TurnState::Cancelling)
}

pub(crate) fn turn_state_for_snapshot(pending_permissions: &[(String, String)]) -> TurnState {
    if pending_permissions.is_empty() {
        TurnState::Idle
    } else {
        TurnState::AwaitingPermission
    }
}

#[cfg(test)]
mod tests {
    use super::{
        TurnState, session_composer_cancel_visible, session_composer_disabled,
        session_composer_status_message,
    };

    #[test]
    fn session_composer_is_disabled_while_a_reply_is_pending() {
        assert!(session_composer_disabled(
            "active",
            TurnState::AwaitingReply,
        ));
    }

    #[test]
    fn session_composer_prompts_for_permission_resolution_before_new_messages() {
        assert_eq!(
            session_composer_status_message("active", TurnState::AwaitingPermission),
            "Resolve the request below before sending another message."
        );
    }

    #[test]
    fn active_session_hides_idle_status_copy() {
        assert_eq!(
            session_composer_status_message("active", TurnState::Idle),
            ""
        );
    }

    #[test]
    fn pending_turns_show_the_cancel_action() {
        assert!(session_composer_cancel_visible(
            TurnState::AwaitingReply,
            false,
        ));
        assert!(!session_composer_cancel_visible(
            TurnState::AwaitingPermission,
            true,
        ));
    }
}
