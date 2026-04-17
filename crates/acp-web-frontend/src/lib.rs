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

use acp_contracts::{
    ConversationMessage, MessageRole, PermissionDecision, PermissionRequest, SessionSnapshot,
    SessionStatus, StreamEvent, StreamEventPayload,
};
use futures_util::StreamExt;
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingPermission {
    pub request_id: String,
    pub summary: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionLifecycle {
    Loading,
    Active,
    Closed,
    Unavailable,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TurnState {
    Idle,
    Submitting,
    AwaitingReply,
    AwaitingPermission,
    Cancelling,
}

struct SessionBootstrap {
    entries: Vec<TranscriptEntry>,
    pending_permissions: Vec<PendingPermission>,
    session_status: SessionLifecycle,
}

#[derive(Clone, Copy)]
struct SessionSignals {
    entries: RwSignal<Vec<TranscriptEntry>>,
    pending_permissions: RwSignal<Vec<PendingPermission>>,
    action_error: RwSignal<Option<String>>,
    connection_error: RwSignal<Option<String>>,
    session_status: RwSignal<SessionLifecycle>,
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
        signals.action_error,
        signals.draft,
    );
    let (on_approve, on_deny, on_cancel) = session_permission_callbacks(
        session_id.clone(),
        signals.turn_state,
        signals.pending_permissions,
        signals.pending_action_busy,
        signals.action_error,
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
        action_error: RwSignal::new(None::<String>),
        connection_error: RwSignal::new(None::<String>),
        session_status: RwSignal::new(SessionLifecycle::Loading),
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
    let action_error = signals.action_error;
    let connection_error = signals.connection_error;
    let combined_error = Signal::derive(move || action_error.get().or(connection_error.get()));
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
            <SessionTopBar message=combined_error />
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
fn SessionTopBar(#[prop(into)] message: Signal<Option<String>>) -> impl IntoView {
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
    #[prop(into)] pending_permissions: Signal<Vec<PendingPermission>>,
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
    pending_permissions: RwSignal<Vec<PendingPermission>>,
    pending_action_busy: RwSignal<bool>,
    action_error: RwSignal<Option<String>>,
) -> (Callback<String>, Callback<String>, Callback<()>) {
    (
        permission_resolution_callback(
            session_id.clone(),
            PermissionDecision::Approve,
            turn_state,
            pending_permissions,
            pending_action_busy,
            action_error,
        ),
        permission_resolution_callback(
            session_id.clone(),
            PermissionDecision::Deny,
            turn_state,
            pending_permissions,
            pending_action_busy,
            action_error,
        ),
        cancel_turn_callback(
            session_id,
            turn_state,
            pending_permissions,
            pending_action_busy,
            action_error,
        ),
    )
}

fn spawn_session_bootstrap(session_id: String, signals: SessionSignals) {
    leptos::task::spawn_local(async move {
        match api::load_session(&session_id).await {
            Ok(session) => {
                let is_closed = session.status == SessionStatus::Closed;
                apply_loaded_session(session, signals);
                if !is_closed {
                    subscribe_sse(&session_id, signals).await;
                }
            }
            Err(api::SessionLoadError::ResumeUnavailable(message)) => {
                record_session_bootstrap_failure(message, SessionLifecycle::Unavailable, signals);
            }
            Err(api::SessionLoadError::Other(message)) => {
                record_session_bootstrap_failure(message, SessionLifecycle::Error, signals);
            }
        }
    });
}

async fn subscribe_sse(session_id: &str, signals: SessionSignals) {
    let (event_source, mut rx) = match api::open_session_event_stream(session_id) {
        Ok(stream) => stream,
        Err(message) => {
            signals.connection_error.set(Some(message));
            return;
        }
    };

    while let Some(item) = rx.next().await {
        match item {
            api::SseItem::Event(event) => {
                signals.connection_error.set(None);
                handle_sse_event(event, signals);
                if matches!(
                    signals.session_status.get_untracked(),
                    SessionLifecycle::Closed
                ) {
                    event_source.close();
                    return;
                }
            }
            api::SseItem::Disconnected => {
                signals.connection_error.set(Some(
                    "Event stream disconnected; reconnecting...".to_string(),
                ));
            }
            api::SseItem::ParseError(message) => {
                signals.connection_error.set(Some(message));
                event_source.close();
                return;
            }
        }
    }

    event_source.close();
}

fn session_submit_callback(
    session_id: String,
    turn_state: RwSignal<TurnState>,
    action_error: RwSignal<Option<String>>,
    draft: RwSignal<String>,
) -> Callback<String> {
    Callback::new(move |prompt: String| {
        let session_id = session_id.clone();
        turn_state.set(TurnState::Submitting);
        action_error.set(None);
        leptos::task::spawn_local(async move {
            match api::send_message(&session_id, &prompt).await {
                Ok(()) => {
                    clear_prepared_session_id();
                    draft.set(String::new());
                    turn_state.set(TurnState::AwaitingReply);
                }
                Err(message) => {
                    action_error.set(Some(message));
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

fn apply_loaded_session(session: SessionSnapshot, signals: SessionSignals) {
    let bootstrap = session_bootstrap_from_snapshot(session);
    let turn_state_for_session = turn_state_for_snapshot(&bootstrap.pending_permissions);
    let should_clear_prepared_session =
        matches!(bootstrap.session_status, SessionLifecycle::Closed)
            || bootstrap
                .entries
                .iter()
                .any(|entry| matches!(entry.role, EntryRole::User));

    signals.entries.set(bootstrap.entries);
    signals
        .pending_permissions
        .set(bootstrap.pending_permissions);
    signals.session_status.set(bootstrap.session_status);
    signals.turn_state.set(turn_state_for_session);
    if should_clear_prepared_session {
        clear_prepared_session_id();
    }
}

fn record_session_bootstrap_failure(
    message: String,
    session_lifecycle: SessionLifecycle,
    signals: SessionSignals,
) {
    clear_prepared_session_id();
    signals.connection_error.set(Some(message));
    signals.session_status.set(session_lifecycle);
    signals.turn_state.set(TurnState::Idle);
}

fn permission_resolution_callback(
    session_id: String,
    decision: PermissionDecision,
    turn_state: RwSignal<TurnState>,
    pending_permissions: RwSignal<Vec<PendingPermission>>,
    pending_action_busy: RwSignal<bool>,
    action_error: RwSignal<Option<String>>,
) -> Callback<String> {
    Callback::new(move |request_id: String| {
        let session_id = session_id.clone();
        let request_id_for_state = request_id.clone();
        let request_id_for_api = request_id.clone();
        let decision = decision.clone();
        let request_decision = decision.clone();
        pending_action_busy.set(true);
        action_error.set(None);
        leptos::task::spawn_local(async move {
            match api::resolve_permission(&session_id, &request_id_for_api, request_decision).await
            {
                Ok(_) => {
                    pending_permissions.update(|current_permissions| {
                        current_permissions.retain(|current_permission| {
                            current_permission.request_id.as_str() != request_id_for_state.as_str()
                        });
                    });
                    turn_state.set(match decision {
                        PermissionDecision::Approve => TurnState::AwaitingReply,
                        PermissionDecision::Deny => TurnState::Idle,
                    });
                }
                Err(message) => {
                    action_error.set(Some(message));
                }
            }
            pending_action_busy.set(false);
        });
    })
}

fn cancel_turn_callback(
    session_id: String,
    turn_state: RwSignal<TurnState>,
    pending_permissions: RwSignal<Vec<PendingPermission>>,
    pending_action_busy: RwSignal<bool>,
    action_error: RwSignal<Option<String>>,
) -> Callback<()> {
    Callback::new(move |()| {
        let session_id = session_id.clone();
        let previous_turn_state = turn_state.get_untracked();
        pending_action_busy.set(true);
        turn_state.set(TurnState::Cancelling);
        action_error.set(None);
        leptos::task::spawn_local(async move {
            match api::cancel_turn(&session_id).await {
                Ok(cancelled) if cancelled.cancelled => {
                    pending_permissions.set(Vec::new());
                    turn_state.set(TurnState::Idle);
                }
                Ok(_) => {
                    action_error.set(Some("No running turn is active.".to_string()));
                    if turn_state.get_untracked() == TurnState::Cancelling {
                        turn_state.set(previous_turn_state);
                    }
                }
                Err(message) => {
                    action_error.set(Some(message));
                    if turn_state.get_untracked() == TurnState::Cancelling {
                        turn_state.set(previous_turn_state);
                    }
                }
            }
            pending_action_busy.set(false);
        });
    })
}

fn handle_sse_event(event: StreamEvent, signals: SessionSignals) {
    let StreamEvent { sequence, payload } = event;

    match payload {
        StreamEventPayload::SessionSnapshot { session } => apply_session_snapshot(session, signals),
        StreamEventPayload::ConversationMessage { message } => {
            apply_conversation_message(message, signals)
        }
        StreamEventPayload::PermissionRequested { request } => {
            apply_permission_request(request, signals)
        }
        StreamEventPayload::SessionClosed { reason, .. } => {
            apply_session_closed(sequence, reason, signals)
        }
        StreamEventPayload::Status { message } => apply_status_update(sequence, message, signals),
    }
}

fn apply_session_snapshot(session: SessionSnapshot, signals: SessionSignals) {
    let bootstrap = session_bootstrap_from_snapshot(session);
    signals.session_status.set(bootstrap.session_status);
    if should_apply_snapshot_turn_state(signals.turn_state.get_untracked()) {
        signals
            .turn_state
            .set(turn_state_for_snapshot(&bootstrap.pending_permissions));
    }
    signals
        .pending_permissions
        .set(bootstrap.pending_permissions);
    signals.entries.set(bootstrap.entries);
}

fn apply_conversation_message(message: ConversationMessage, signals: SessionSignals) {
    let is_assistant_message = matches!(message.role, MessageRole::Assistant);
    let mut appended = false;
    signals.entries.update(|current_entries| {
        if !current_entries.iter().any(|entry| entry.id == message.id) {
            appended = true;
            current_entries.push(message_to_entry(message));
        }
    });
    if appended
        && is_assistant_message
        && should_release_turn_state(signals.turn_state.get_untracked())
    {
        signals.turn_state.set(TurnState::Idle);
    }
}

fn apply_permission_request(request: PermissionRequest, signals: SessionSignals) {
    let request_id = request.request_id;
    let summary = request.summary;
    signals.pending_permissions.update(|current_permissions| {
        if !current_permissions
            .iter()
            .any(|current_permission| current_permission.request_id.as_str() == request_id.as_str())
        {
            current_permissions.push(PendingPermission {
                request_id: request_id.clone(),
                summary: summary.clone(),
            });
        }
    });
    signals.turn_state.set(TurnState::AwaitingPermission);
}

fn apply_session_closed(sequence: u64, reason: String, signals: SessionSignals) {
    signals.session_status.set(SessionLifecycle::Closed);
    signals.turn_state.set(TurnState::Idle);
    push_status_entry(signals.entries, sequence, reason);
}

fn apply_status_update(sequence: u64, message: String, signals: SessionSignals) {
    if should_release_turn_state(signals.turn_state.get_untracked()) {
        signals.turn_state.set(TurnState::Idle);
    }
    push_status_entry(signals.entries, sequence, message);
}

fn session_bootstrap_from_snapshot(session: SessionSnapshot) -> SessionBootstrap {
    let SessionSnapshot {
        status,
        messages,
        pending_permissions,
        ..
    } = session;

    SessionBootstrap {
        entries: messages.into_iter().map(message_to_entry).collect(),
        pending_permissions: pending_permissions_to_items(pending_permissions),
        session_status: session_status_label(status),
    }
}

fn pending_permissions_to_items(
    pending_permissions: Vec<PermissionRequest>,
) -> Vec<PendingPermission> {
    pending_permissions
        .into_iter()
        .map(|request| PendingPermission {
            request_id: request.request_id,
            summary: request.summary,
        })
        .collect()
}

fn push_status_entry(entries: RwSignal<Vec<TranscriptEntry>>, sequence: u64, text: String) {
    if text.trim().is_empty() {
        return;
    }

    let entry_id = format!("status-{sequence}");
    entries.update(|current_entries| {
        if current_entries.iter().any(|entry| entry.id == entry_id) {
            return;
        }

        current_entries.push(TranscriptEntry {
            id: entry_id.clone(),
            role: EntryRole::Status,
            text: text.clone(),
        });
    });
}

fn message_to_entry(message: ConversationMessage) -> TranscriptEntry {
    TranscriptEntry {
        id: message.id,
        role: message_role(message.role),
        text: message.text,
    }
}

fn message_role(role: MessageRole) -> EntryRole {
    match role {
        MessageRole::User => EntryRole::User,
        MessageRole::Assistant => EntryRole::Assistant,
    }
}

fn session_status_label(status: SessionStatus) -> SessionLifecycle {
    match status {
        SessionStatus::Active => SessionLifecycle::Active,
        SessionStatus::Closed => SessionLifecycle::Closed,
    }
}

fn session_composer_disabled_signal(
    turn_state: RwSignal<TurnState>,
    session_status: RwSignal<SessionLifecycle>,
) -> Signal<bool> {
    Signal::derive(move || session_composer_disabled(session_status.get(), turn_state.get()))
}

fn session_composer_status_signal(
    turn_state: RwSignal<TurnState>,
    session_status: RwSignal<SessionLifecycle>,
) -> Signal<String> {
    Signal::derive(move || session_composer_status_message(session_status.get(), turn_state.get()))
}

fn session_composer_cancel_visible_signal(
    turn_state: RwSignal<TurnState>,
    pending_permissions: RwSignal<Vec<PendingPermission>>,
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

fn session_composer_disabled(session_status: SessionLifecycle, turn_state: TurnState) -> bool {
    session_status != SessionLifecycle::Active || turn_state != TurnState::Idle
}

fn session_composer_status_message(
    session_status: SessionLifecycle,
    turn_state: TurnState,
) -> String {
    match turn_state {
        TurnState::Submitting | TurnState::AwaitingReply => "Waiting for response...".to_string(),
        TurnState::AwaitingPermission => {
            "Resolve the request below before sending another message.".to_string()
        }
        TurnState::Cancelling => "Cancelling...".to_string(),
        TurnState::Idle => match session_status {
            SessionLifecycle::Active => String::new(),
            SessionLifecycle::Closed => "This session is closed.".to_string(),
            SessionLifecycle::Loading => "Connecting...".to_string(),
            SessionLifecycle::Unavailable | SessionLifecycle::Error => {
                "Session unavailable. Start a fresh chat.".to_string()
            }
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

pub(crate) fn should_release_turn_state(current: TurnState) -> bool {
    matches!(current, TurnState::AwaitingReply | TurnState::Cancelling)
}

pub(crate) fn turn_state_for_snapshot(pending_permissions: &[PendingPermission]) -> TurnState {
    if pending_permissions.is_empty() {
        TurnState::Idle
    } else {
        TurnState::AwaitingPermission
    }
}

#[cfg(test)]
mod tests {
    use super::{
        EntryRole, PendingPermission, SessionLifecycle, TurnState, session_bootstrap_from_snapshot,
        session_composer_cancel_visible, session_composer_disabled,
        session_composer_status_message, should_release_turn_state, turn_state_for_snapshot,
    };
    use acp_contracts::{SessionResponse, SessionStatus};

    #[test]
    fn session_composer_is_disabled_while_a_reply_is_pending() {
        assert!(session_composer_disabled(
            SessionLifecycle::Active,
            TurnState::AwaitingReply,
        ));
    }

    #[test]
    fn session_composer_prompts_for_permission_resolution_before_new_messages() {
        assert_eq!(
            session_composer_status_message(
                SessionLifecycle::Active,
                TurnState::AwaitingPermission,
            ),
            "Resolve the request below before sending another message."
        );
    }

    #[test]
    fn active_session_hides_idle_status_copy() {
        assert_eq!(
            session_composer_status_message(SessionLifecycle::Active, TurnState::Idle),
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

    #[test]
    fn should_release_turn_state_when_reply_or_cancel_finishes() {
        assert!(should_release_turn_state(TurnState::AwaitingReply));
        assert!(should_release_turn_state(TurnState::Cancelling));
        assert!(!should_release_turn_state(TurnState::Idle));
    }

    #[test]
    fn turn_state_for_snapshot_uses_typed_pending_permissions() {
        assert_eq!(turn_state_for_snapshot(&[]), TurnState::Idle);
        assert_eq!(
            turn_state_for_snapshot(&[PendingPermission {
                request_id: "req_1".to_string(),
                summary: "read README.md".to_string(),
            }]),
            TurnState::AwaitingPermission
        );
    }

    #[test]
    fn session_bootstrap_from_snapshot_maps_messages_and_permissions() {
        let body = serde_json::json!({
            "session": {
                "id": "s_123",
                "status": "closed",
                "latest_sequence": 8,
                "messages": [
                    {
                        "id": "m_user",
                        "role": "user",
                        "text": "hello",
                        "created_at": "2026-04-17T01:00:00Z"
                    },
                    {
                        "id": "m_assistant",
                        "role": "assistant",
                        "text": "world",
                        "created_at": "2026-04-17T01:00:01Z"
                    }
                ],
                "pending_permissions": [{
                    "request_id": "req_1",
                    "summary": "read README.md"
                }]
            }
        })
        .to_string();

        let bootstrap = session_bootstrap_from_snapshot(
            serde_json::from_str::<SessionResponse>(&body)
                .expect("wrapped session payload should decode")
                .session,
        );

        assert_eq!(bootstrap.session_status, SessionLifecycle::Closed);
        assert_eq!(bootstrap.entries.len(), 2);
        assert_eq!(bootstrap.entries[0].id, "m_user");
        assert!(matches!(bootstrap.entries[0].role, EntryRole::User));
        assert_eq!(bootstrap.entries[1].id, "m_assistant");
        assert!(matches!(bootstrap.entries[1].role, EntryRole::Assistant));
        assert_eq!(
            bootstrap.pending_permissions,
            vec![PendingPermission {
                request_id: "req_1".to_string(),
                summary: "read README.md".to_string(),
            }]
        );
    }

    #[test]
    fn session_status_label_maps_contract_statuses() {
        assert_eq!(
            super::session_status_label(SessionStatus::Active),
            SessionLifecycle::Active
        );
        assert_eq!(
            super::session_status_label(SessionStatus::Closed),
            SessionLifecycle::Closed
        );
    }
}
