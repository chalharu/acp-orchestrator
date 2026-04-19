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
mod slash;

use acp_contracts::{
    CompletionCandidate, ConversationMessage, MessageRole, PermissionDecision, PermissionRequest,
    SessionListItem, SessionSnapshot, SessionStatus, StreamEvent, StreamEventPayload,
};
use futures_util::{
    StreamExt,
    future::{AbortHandle, Abortable},
};
use leptos::{portal::Portal, prelude::*};
use wasm_bindgen::JsCast;
use web_sys::EventSource;

use components::{
    ChatActivity, Composer, ComposerSlashCallbacks, ComposerSlashSignals, ErrorBanner, Transcript,
};
use slash::{
    BrowserSlashAction, apply_slash_completion, cycle_slash_selection, local_browser_commands,
    local_slash_candidates, parse_browser_slash_action, slash_palette_is_visible,
    slash_palette_should_apply_on_enter,
};

const PREPARED_SESSION_STORAGE_KEY: &str = "acp-prepared-session-id";
const DRAFT_STORAGE_KEY_PREFIX: &str = "acp-draft-";
const CLOSED_SESSION_MESSAGE: &str = "Conversation ended.";

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
pub enum BadgeTone {
    Neutral,
    Success,
    Warning,
    Danger,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StatusBadge {
    pub label: &'static str,
    pub value: &'static str,
    pub tone: BadgeTone,
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
struct SlashSignals {
    candidates: RwSignal<Vec<CompletionCandidate>>,
    selected_index: RwSignal<usize>,
}

#[derive(Clone, Copy)]
struct SessionListSignals {
    items: RwSignal<Vec<SessionListItem>>,
    loaded: RwSignal<bool>,
    error: RwSignal<Option<String>>,
    deleting_id: RwSignal<Option<String>>,
    renaming_id: RwSignal<Option<String>>,
    saving_rename_id: RwSignal<Option<String>>,
    rename_draft: RwSignal<String>,
}

#[derive(Clone, Copy)]
struct SessionSignals {
    entries: RwSignal<Vec<TranscriptEntry>>,
    pending_permissions: RwSignal<Vec<PendingPermission>>,
    action_error: RwSignal<Option<String>>,
    connection_error: RwSignal<Option<String>>,
    event_source: RwSignal<Option<EventSource>>,
    stream_abort: RwSignal<Option<AbortHandle>>,
    session_status: RwSignal<SessionLifecycle>,
    turn_state: RwSignal<TurnState>,
    pending_action_busy: RwSignal<bool>,
    draft: RwSignal<String>,
    slash: SlashSignals,
    list: SessionListSignals,
    tool_activity_serial: RwSignal<u64>,
}

#[derive(Clone, Copy)]
struct SessionViewCallbacks {
    submit: Callback<String>,
    approve: Callback<String>,
    deny: Callback<String>,
    cancel: Callback<()>,
    slash: ComposerSlashCallbacks,
    rename_session: Callback<(String, String)>,
    delete_session: Callback<String>,
}

#[derive(Clone, Copy)]
struct SessionComposerSignals {
    disabled: Signal<bool>,
    status: Signal<String>,
    cancel_visible: Signal<bool>,
    cancel_busy: Signal<bool>,
    slash_palette_visible: Signal<bool>,
    slash_candidates: Signal<Vec<CompletionCandidate>>,
    slash_selected_index: Signal<usize>,
    slash_apply_on_enter: Signal<bool>,
}

#[derive(Clone, Copy)]
struct SessionShellSignals {
    sessions: Signal<Vec<SessionListItem>>,
    list: SessionListSignals,
    delete_disabled: Signal<bool>,
}

#[derive(Clone, Copy)]
struct SessionMainSignals {
    session_status: Signal<SessionLifecycle>,
    topbar_message: Signal<Option<String>>,
    connection_badge: Signal<StatusBadge>,
    worker_badge: Signal<StatusBadge>,
    entries: Signal<Vec<TranscriptEntry>>,
    pending_permissions: Signal<Vec<PendingPermission>>,
    pending_action_busy: Signal<bool>,
}

#[derive(Clone, Copy)]
struct SessionSidebarItemSignals {
    is_renaming: Signal<bool>,
    is_deleting: Signal<bool>,
    is_saving_rename: Signal<bool>,
    rename_action_disabled: Signal<bool>,
    delete_action_disabled: Signal<bool>,
    save_rename_disabled: Signal<bool>,
}

#[derive(Clone, Copy)]
struct SessionSidebarItemCallbacks {
    begin_rename: Callback<()>,
    cancel_rename: Callback<()>,
    commit_rename: Callback<()>,
    delete_session: Callback<()>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SidebarSession {
    id: String,
    href: String,
    title: String,
    is_current: bool,
    is_closed: bool,
}

#[component]
fn SessionView(session_id: String) -> impl IntoView {
    let signals = session_signals();
    let sidebar_open = RwSignal::new(default_sidebar_open());
    let current_session_deleting = current_session_deleting_signal(session_id.clone(), signals);
    restore_session_draft(&session_id, signals);
    persist_session_draft(session_id.clone(), signals.draft);
    bind_slash_completion(signals);
    spawn_session_bootstrap(session_id.clone(), signals);

    session_view_content(
        session_id.clone(),
        signals,
        session_composer_signals(signals, current_session_deleting),
        session_view_callbacks(session_id, signals),
        sidebar_open,
    )
}

fn current_session_deleting_signal(session_id: String, signals: SessionSignals) -> Signal<bool> {
    Signal::derive(move || signals.list.deleting_id.get().as_deref() == Some(session_id.as_str()))
}

fn restore_session_draft(session_id: &str, signals: SessionSignals) {
    let stored_draft = load_draft(session_id);
    if !stored_draft.is_empty() {
        signals.draft.set(stored_draft);
    }
}

fn persist_session_draft(session_id: String, draft: RwSignal<String>) {
    Effect::new(move |_| {
        save_draft(&session_id, &draft.get());
    });
}

fn session_composer_signals(
    signals: SessionSignals,
    current_session_deleting: Signal<bool>,
) -> SessionComposerSignals {
    SessionComposerSignals {
        disabled: session_composer_disabled_signal(
            signals.turn_state,
            signals.session_status,
            current_session_deleting,
        ),
        status: session_composer_status_signal(
            signals.turn_state,
            signals.session_status,
            current_session_deleting,
        ),
        cancel_visible: session_composer_cancel_visible_signal(
            signals.turn_state,
            signals.pending_permissions,
            current_session_deleting,
        ),
        cancel_busy: session_composer_cancel_busy_signal(
            signals.turn_state,
            signals.pending_action_busy,
            current_session_deleting,
        ),
        slash_palette_visible: Signal::derive(move || {
            slash_palette_is_visible(&signals.draft.get())
        }),
        slash_candidates: Signal::derive(move || signals.slash.candidates.get()),
        slash_selected_index: Signal::derive(move || signals.slash.selected_index.get()),
        slash_apply_on_enter: Signal::derive(move || {
            slash_palette_should_apply_on_enter(
                &signals.draft.get(),
                &signals.slash.candidates.get(),
                signals.slash.selected_index.get(),
            )
        }),
    }
}

fn session_view_callbacks(session_id: String, signals: SessionSignals) -> SessionViewCallbacks {
    let (approve, deny, cancel) = session_permission_callbacks(session_id.clone(), signals);

    SessionViewCallbacks {
        submit: session_submit_callback(session_id.clone(), signals),
        approve,
        deny,
        cancel,
        slash: slash_palette_callbacks(signals),
        rename_session: rename_session_callback(signals),
        delete_session: delete_session_callback(session_id, signals),
    }
}

fn session_signals() -> SessionSignals {
    SessionSignals {
        entries: RwSignal::new(Vec::new()),
        pending_permissions: RwSignal::new(Vec::new()),
        action_error: RwSignal::new(None::<String>),
        connection_error: RwSignal::new(None::<String>),
        event_source: RwSignal::new(None::<EventSource>),
        stream_abort: RwSignal::new(None::<AbortHandle>),
        session_status: RwSignal::new(SessionLifecycle::Loading),
        turn_state: RwSignal::new(TurnState::Idle),
        pending_action_busy: RwSignal::new(false),
        draft: RwSignal::new(String::new()),
        slash: SlashSignals {
            candidates: RwSignal::new(Vec::new()),
            selected_index: RwSignal::new(0),
        },
        list: SessionListSignals {
            items: RwSignal::new(Vec::new()),
            loaded: RwSignal::new(false),
            error: RwSignal::new(None::<String>),
            deleting_id: RwSignal::new(None::<String>),
            renaming_id: RwSignal::new(None::<String>),
            saving_rename_id: RwSignal::new(None::<String>),
            rename_draft: RwSignal::new(String::new()),
        },
        tool_activity_serial: RwSignal::new(0),
    }
}

fn session_view_content(
    current_session_id: String,
    signals: SessionSignals,
    composer: SessionComposerSignals,
    callbacks: SessionViewCallbacks,
    sidebar_open: RwSignal<bool>,
) -> impl IntoView {
    let draft = signals.draft;
    let shell_signals = session_shell_signals(signals);
    let main_signals = session_main_signals(signals);

    view! {
        <SessionBackdrop sidebar_open=sidebar_open />
        <main class="app-shell app-shell--session">
            <SessionShell
                current_session_id=current_session_id
                sidebar_open=sidebar_open
                shell_signals=shell_signals
                main_signals=main_signals
                composer=composer
                callbacks=callbacks
                draft=draft
            />
        </main>
    }
}

fn session_shell_signals(signals: SessionSignals) -> SessionShellSignals {
    let session_list = signals.list.items;
    let pending_action_busy = signals.pending_action_busy;

    SessionShellSignals {
        sessions: Signal::derive(move || session_list.get()),
        list: signals.list,
        delete_disabled: Signal::derive(move || {
            session_action_busy(signals.turn_state.get(), pending_action_busy.get(), false)
        }),
    }
}

fn session_main_signals(signals: SessionSignals) -> SessionMainSignals {
    let entries = signals.entries;
    let pending_action_busy = signals.pending_action_busy;
    let action_error = signals.action_error;
    let connection_error = signals.connection_error;
    let pending_permissions = signals.pending_permissions;
    let session_status = signals.session_status;
    let turn_state = signals.turn_state;

    SessionMainSignals {
        session_status: Signal::derive(move || session_status.get()),
        topbar_message: Signal::derive(move || action_error.get().or(connection_error.get())),
        connection_badge: Signal::derive(move || {
            connection_badge_state(session_status.get(), connection_error.get().is_some())
        }),
        worker_badge: Signal::derive(move || {
            worker_badge_state(
                session_status.get(),
                turn_state.get(),
                !pending_permissions.get().is_empty(),
            )
        }),
        entries: Signal::derive(move || entries.get()),
        pending_permissions: Signal::derive(move || pending_permissions.get()),
        pending_action_busy: Signal::derive(move || pending_action_busy.get()),
    }
}

#[component]
fn SessionShell(
    current_session_id: String,
    sidebar_open: RwSignal<bool>,
    shell_signals: SessionShellSignals,
    main_signals: SessionMainSignals,
    composer: SessionComposerSignals,
    callbacks: SessionViewCallbacks,
    draft: RwSignal<String>,
) -> impl IntoView {
    let SessionViewCallbacks {
        rename_session: on_rename_session,
        delete_session: on_delete_session,
        ..
    } = callbacks;

    view! {
        <div class=move || session_layout_class(sidebar_open.get())>
            <SessionSidebar
                current_session_id=current_session_id
                sessions=shell_signals.sessions
                session_list_loaded=shell_signals.list.loaded
                session_list_error=shell_signals.list.error
                sidebar_open=sidebar_open
                deleting_session_id=shell_signals.list.deleting_id
                delete_disabled=shell_signals.delete_disabled
                renaming_session_id=shell_signals.list.renaming_id
                saving_rename_session_id=shell_signals.list.saving_rename_id
                rename_draft=shell_signals.list.rename_draft
                on_rename_session=on_rename_session
                on_delete_session=on_delete_session
            />
            <SessionMain
                main_signals=main_signals
                sidebar_open=sidebar_open
                composer=composer
                callbacks=callbacks
                draft=draft
            />
        </div>
    }
}

#[component]
fn SessionBackdrop(sidebar_open: RwSignal<bool>) -> impl IntoView {
    view! {
        <Portal>
            <div
                class="session-layout__backdrop"
                role="button"
                aria-label="Close session sidebar"
                tabindex="0"
                hidden=move || !sidebar_open.get()
                on:click=move |_| sidebar_open.set(false)
                on:keydown=move |ev: web_sys::KeyboardEvent| {
                    if matches!(ev.key().as_str(), "Enter" | " " | "Spacebar") {
                        ev.prevent_default();
                        sidebar_open.set(false);
                    }
                }
            ></div>
        </Portal>
    }
}

#[component]
fn SessionMain(
    main_signals: SessionMainSignals,
    sidebar_open: RwSignal<bool>,
    composer: SessionComposerSignals,
    callbacks: SessionViewCallbacks,
    draft: RwSignal<String>,
) -> impl IntoView {
    view! {
        <section class="session-main">
            <SessionTopBar
                message=main_signals.topbar_message
                connection_badge=main_signals.connection_badge
                worker_badge=main_signals.worker_badge
                sidebar_open=sidebar_open
            />
            <SessionTranscriptPanel
                entries=main_signals.entries
                session_status=main_signals.session_status
                pending_permissions=main_signals.pending_permissions
                pending_action_busy=main_signals.pending_action_busy
                on_approve=callbacks.approve
                on_deny=callbacks.deny
                on_cancel=callbacks.cancel
            />
            <SessionDock composer=composer callbacks=callbacks draft=draft />
        </section>
    }
}

#[component]
fn SessionTranscriptPanel(
    #[prop(into)] entries: Signal<Vec<TranscriptEntry>>,
    #[prop(into)] session_status: Signal<SessionLifecycle>,
    #[prop(into)] pending_permissions: Signal<Vec<PendingPermission>>,
    #[prop(into)] pending_action_busy: Signal<bool>,
    on_approve: Callback<String>,
    on_deny: Callback<String>,
    on_cancel: Callback<()>,
) -> impl IntoView {
    view! {
        <div class="chat-body">
            <Transcript entries=entries />
            <ChatActivity
                items=pending_permissions
                busy=pending_action_busy
                on_approve=on_approve
                on_deny=on_deny
                on_cancel=on_cancel
            />
        </div>
        <SessionClosedNotice session_status=session_status />
    }
}

#[component]
fn SessionClosedNotice(#[prop(into)] session_status: Signal<SessionLifecycle>) -> impl IntoView {
    view! {
        <Show when=move || matches!(session_status.get(), SessionLifecycle::Closed)>
            <div class="session-ended-notice" role="status">
                <p class="session-ended-notice__text">
                    "This conversation has ended. "
                    <a href="/app/">"Start a new chat."</a>
                </p>
            </div>
        </Show>
    }
}

#[component]
fn SessionTopBar(
    #[prop(into)] message: Signal<Option<String>>,
    #[prop(into)] connection_badge: Signal<StatusBadge>,
    #[prop(into)] worker_badge: Signal<StatusBadge>,
    sidebar_open: RwSignal<bool>,
) -> impl IntoView {
    view! {
        <div class="chat-topbar">
            <div class="chat-topbar__controls">
                <button
                    class="session-sidebar__toggle"
                    type="button"
                    aria-expanded=move || if sidebar_open.get() { "true" } else { "false" }
                    on:click=move |_| sidebar_open.update(|open| *open = !*open)
                >
                    <span class="sidebar-toggle-icon" aria-hidden="true">
                        {move || if sidebar_open.get() { "←" } else { "☰" }}
                    </span>
                    <span class="session-sidebar__toggle-label">
                        {move || if sidebar_open.get() { "Hide sessions" } else { "Show sessions" }}
                    </span>
                </button>
                <div class="chat-topbar__badges" aria-label="Connection and worker state">
                    <StatusBadgeView badge=connection_badge />
                    <StatusBadgeView badge=worker_badge />
                </div>
            </div>
            <ErrorBanner message=message />
        </div>
    }
}

#[component]
fn StatusBadgeView(#[prop(into)] badge: Signal<StatusBadge>) -> impl IntoView {
    view! {
        <p class=move || status_badge_class(badge.get())>
            <span class="status-badge__label">{move || badge.get().label}</span>
            <span class="status-badge__value">{move || badge.get().value}</span>
        </p>
    }
}

#[component]
fn SessionSidebar(
    current_session_id: String,
    #[prop(into)] sessions: Signal<Vec<SessionListItem>>,
    #[prop(into)] session_list_loaded: Signal<bool>,
    #[prop(into)] session_list_error: Signal<Option<String>>,
    sidebar_open: RwSignal<bool>,
    #[prop(into)] deleting_session_id: Signal<Option<String>>,
    #[prop(into)] delete_disabled: Signal<bool>,
    renaming_session_id: RwSignal<Option<String>>,
    #[prop(into)] saving_rename_session_id: Signal<Option<String>>,
    rename_draft: RwSignal<String>,
    on_rename_session: Callback<(String, String)>,
    on_delete_session: Callback<String>,
) -> impl IntoView {
    let session_items =
        Signal::derive(move || sidebar_sessions(&sessions.get(), &current_session_id));

    view! {
        <aside class=move || session_sidebar_class(sidebar_open.get())>
            <SessionSidebarHeader sidebar_open=sidebar_open />
            <SessionSidebarStatus session_list_error=session_list_error session_items=session_items />
            <SessionSidebarNav
                session_items=session_items
                session_list_loaded=session_list_loaded
                session_list_error=session_list_error
                deleting_session_id=deleting_session_id
                delete_disabled=delete_disabled
                renaming_session_id=renaming_session_id
                saving_rename_session_id=saving_rename_session_id
                rename_draft=rename_draft
                on_rename_session=on_rename_session
                on_delete_session=on_delete_session
            />
        </aside>
    }
}

#[component]
fn SessionSidebarHeader(sidebar_open: RwSignal<bool>) -> impl IntoView {
    view! {
        <div class="session-sidebar__header">
            <a class="session-sidebar__new-link" href="/app/" aria-label="New chat">
                <span class="session-sidebar__new-link-icon" aria-hidden="true">
                    "+"
                </span>
                <span class="session-sidebar__new-link-label">"New chat"</span>
            </a>
            <button
                type="button"
                class="session-sidebar__dismiss"
                on:click=move |_| sidebar_open.set(false)
            >
                <span aria-hidden="true">{"✕"}</span>
                <span class="sr-only">"Close sidebar"</span>
            </button>
        </div>
    }
}

#[component]
fn SessionSidebarStatus(
    #[prop(into)] session_list_error: Signal<Option<String>>,
    #[prop(into)] session_items: Signal<Vec<SidebarSession>>,
) -> impl IntoView {
    view! {
        <Show when=move || session_list_error.get().is_some() && !session_items.get().is_empty()>
            <p class="session-sidebar__status muted">
                {move || session_list_error.get().unwrap_or_default()}
            </p>
        </Show>
    }
}

#[component]
fn SessionSidebarNav(
    #[prop(into)] session_items: Signal<Vec<SidebarSession>>,
    #[prop(into)] session_list_loaded: Signal<bool>,
    #[prop(into)] session_list_error: Signal<Option<String>>,
    #[prop(into)] deleting_session_id: Signal<Option<String>>,
    #[prop(into)] delete_disabled: Signal<bool>,
    renaming_session_id: RwSignal<Option<String>>,
    #[prop(into)] saving_rename_session_id: Signal<Option<String>>,
    rename_draft: RwSignal<String>,
    on_rename_session: Callback<(String, String)>,
    on_delete_session: Callback<String>,
) -> impl IntoView {
    view! {
        <nav class="session-sidebar__nav" aria-label="Sessions">
            <Show
                when=move || session_list_loaded.get()
                fallback=|| {
                    view! { <p class="session-sidebar__empty muted">"Loading sessions..."</p> }
                }
            >
                <Show
                    when=move || !session_items.get().is_empty()
                    fallback=move || {
                        view! {
                            <p class="session-sidebar__empty muted">
                                {move || session_sidebar_empty_message(session_list_error.get().is_some())}
                            </p>
                        }
                    }
                >
                    <SessionSidebarList
                        session_items=session_items
                        deleting_session_id=deleting_session_id
                        delete_disabled=delete_disabled
                        renaming_session_id=renaming_session_id
                        saving_rename_session_id=saving_rename_session_id
                        rename_draft=rename_draft
                        on_rename_session=on_rename_session
                        on_delete_session=on_delete_session
                    />
                </Show>
            </Show>
        </nav>
    }
}

#[component]
fn SessionSidebarList(
    #[prop(into)] session_items: Signal<Vec<SidebarSession>>,
    #[prop(into)] deleting_session_id: Signal<Option<String>>,
    #[prop(into)] delete_disabled: Signal<bool>,
    renaming_session_id: RwSignal<Option<String>>,
    #[prop(into)] saving_rename_session_id: Signal<Option<String>>,
    rename_draft: RwSignal<String>,
    on_rename_session: Callback<(String, String)>,
    on_delete_session: Callback<String>,
) -> impl IntoView {
    view! {
        <ul class="session-sidebar__list">
            <For
                each=move || session_items.get()
                key=|item| {
                    (
                        item.id.clone(),
                        item.title.clone(),
                        item.is_closed,
                        item.is_current,
                    )
                }
                children=move |item| {
                    view! {
                        <SessionSidebarItem
                            item=item
                            deleting_session_id=deleting_session_id
                            delete_disabled=delete_disabled
                            renaming_session_id=renaming_session_id
                            saving_rename_session_id=saving_rename_session_id
                            rename_draft=rename_draft
                            on_rename_session=on_rename_session
                            on_delete_session=on_delete_session
                        />
                    }
                }
            />
        </ul>
    }
}

fn session_sidebar_item_signals(
    session_id: String,
    is_current: bool,
    deleting_session_id: Signal<Option<String>>,
    delete_disabled: Signal<bool>,
    renaming_session_id: RwSignal<Option<String>>,
    saving_rename_session_id: Signal<Option<String>>,
    rename_draft: RwSignal<String>,
) -> SessionSidebarItemSignals {
    let is_renaming = sidebar_rw_session_match_signal(session_id.clone(), renaming_session_id);
    let is_deleting = sidebar_session_match_signal(session_id.clone(), deleting_session_id);
    let is_saving_rename = sidebar_session_match_signal(session_id, saving_rename_session_id);
    let rename_action_disabled =
        sidebar_rename_action_disabled_signal(is_deleting, saving_rename_session_id);
    let delete_action_disabled = sidebar_delete_action_disabled_signal(
        is_deleting,
        deleting_session_id,
        saving_rename_session_id,
        is_current,
        delete_disabled,
    );
    let save_rename_disabled = sidebar_save_rename_disabled_signal(is_saving_rename, rename_draft);

    SessionSidebarItemSignals {
        is_renaming,
        is_deleting,
        is_saving_rename,
        rename_action_disabled,
        delete_action_disabled,
        save_rename_disabled,
    }
}

fn sidebar_rw_session_match_signal(
    session_id: String,
    active_session_id: RwSignal<Option<String>>,
) -> Signal<bool> {
    Signal::derive(move || active_session_id.get().as_deref() == Some(session_id.as_str()))
}

fn sidebar_session_match_signal(
    session_id: String,
    active_session_id: Signal<Option<String>>,
) -> Signal<bool> {
    Signal::derive(move || active_session_id.get().as_deref() == Some(session_id.as_str()))
}

fn sidebar_rename_action_disabled_signal(
    is_deleting: Signal<bool>,
    saving_rename_session_id: Signal<Option<String>>,
) -> Signal<bool> {
    Signal::derive(move || is_deleting.get() || saving_rename_session_id.get().is_some())
}

fn sidebar_delete_action_disabled_signal(
    is_deleting: Signal<bool>,
    deleting_session_id: Signal<Option<String>>,
    saving_rename_session_id: Signal<Option<String>>,
    is_current: bool,
    delete_disabled: Signal<bool>,
) -> Signal<bool> {
    Signal::derive(move || {
        is_deleting.get()
            || deleting_session_id.get().is_some()
            || saving_rename_session_id.get().is_some()
            || (is_current && delete_disabled.get())
    })
}

fn sidebar_save_rename_disabled_signal(
    is_saving_rename: Signal<bool>,
    rename_draft: RwSignal<String>,
) -> Signal<bool> {
    Signal::derive(move || is_saving_rename.get() || rename_draft.get().trim().is_empty())
}

fn session_sidebar_item_callbacks(
    session_id: String,
    title_for_rename_init: String,
    rename_draft: RwSignal<String>,
    renaming_session_id: RwSignal<Option<String>>,
    is_saving_rename: Signal<bool>,
    on_rename_session: Callback<(String, String)>,
    on_delete_session: Callback<String>,
) -> SessionSidebarItemCallbacks {
    let begin_rename = {
        let session_id = session_id.clone();
        Callback::new(move |()| {
            rename_draft.set(title_for_rename_init.clone());
            renaming_session_id.set(Some(session_id.clone()));
        })
    };
    let cancel_rename = Callback::new(move |()| {
        rename_draft.set(String::new());
        renaming_session_id.set(None);
    });
    let commit_rename = {
        let session_id = session_id.clone();
        Callback::new(move |()| {
            if is_saving_rename.get_untracked() {
                return;
            }
            let title = rename_draft.get_untracked().trim().to_string();
            if !title.is_empty() {
                on_rename_session.run((session_id.clone(), title));
            } else {
                renaming_session_id.set(None);
            }
        })
    };
    let delete_session = Callback::new(move |()| on_delete_session.run(session_id.clone()));

    SessionSidebarItemCallbacks {
        begin_rename,
        cancel_rename,
        commit_rename,
        delete_session,
    }
}

fn session_sidebar_item_view(
    href: String,
    title: String,
    is_current: bool,
    is_closed: bool,
    rename_draft: RwSignal<String>,
    item_signals: SessionSidebarItemSignals,
    callbacks: SessionSidebarItemCallbacks,
) -> impl IntoView {
    view! {
        <li class=move || session_sidebar_item_class(is_current, is_closed)>
            <Show
                when=move || item_signals.is_renaming.get()
                fallback={
                    let href = href.clone();
                    let title = title.clone();
                    move || {
                        view! {
                            <SessionSidebarItemDisplay
                                href=href.clone()
                                title=title.clone()
                                is_current=is_current
                                is_deleting=item_signals.is_deleting
                                rename_action_disabled=item_signals.rename_action_disabled
                                delete_action_disabled=item_signals.delete_action_disabled
                                on_begin_rename=callbacks.begin_rename
                                on_delete=callbacks.delete_session
                            />
                        }
                    }
                }
            >
                <SessionSidebarRenameForm
                    rename_draft=rename_draft
                    is_saving_rename=item_signals.is_saving_rename
                    save_disabled=item_signals.save_rename_disabled
                    on_commit_rename=callbacks.commit_rename
                    on_cancel_rename=callbacks.cancel_rename
                />
            </Show>
        </li>
    }
}

#[component]
fn SessionSidebarItem(
    item: SidebarSession,
    #[prop(into)] deleting_session_id: Signal<Option<String>>,
    #[prop(into)] delete_disabled: Signal<bool>,
    renaming_session_id: RwSignal<Option<String>>,
    #[prop(into)] saving_rename_session_id: Signal<Option<String>>,
    rename_draft: RwSignal<String>,
    on_rename_session: Callback<(String, String)>,
    on_delete_session: Callback<String>,
) -> impl IntoView {
    let item_signals = session_sidebar_item_signals(
        item.id.clone(),
        item.is_current,
        deleting_session_id,
        delete_disabled,
        renaming_session_id,
        saving_rename_session_id,
        rename_draft,
    );
    let callbacks = session_sidebar_item_callbacks(
        item.id,
        item.title.clone(),
        rename_draft,
        renaming_session_id,
        item_signals.is_saving_rename,
        on_rename_session,
        on_delete_session,
    );

    session_sidebar_item_view(
        item.href,
        item.title,
        item.is_current,
        item.is_closed,
        rename_draft,
        item_signals,
        callbacks,
    )
}

#[component]
fn SessionSidebarItemDisplay(
    href: String,
    title: String,
    is_current: bool,
    #[prop(into)] is_deleting: Signal<bool>,
    #[prop(into)] rename_action_disabled: Signal<bool>,
    #[prop(into)] delete_action_disabled: Signal<bool>,
    on_begin_rename: Callback<()>,
    on_delete: Callback<()>,
) -> impl IntoView {
    view! {
        <a
            class="session-sidebar__session-link"
            href=href
            aria-current=if is_current { Some("page") } else { None }
        >
            <span class="session-sidebar__session-title">{title}</span>
        </a>
        <button
            type="button"
            class="session-sidebar__action-btn"
            title="Rename"
            on:click=move |_| on_begin_rename.run(())
            prop:disabled=move || rename_action_disabled.get()
        >
            <span aria-hidden="true">{"✎"}</span>
            <span class="sr-only">"Rename session"</span>
        </button>
        <button
            type="button"
            class="session-sidebar__action-btn session-sidebar__action-btn--danger"
            title="Delete"
            on:click=move |_| on_delete.run(())
            prop:disabled=move || delete_action_disabled.get()
        >
            <Show
                when=move || is_deleting.get()
                fallback=|| view! { <span aria-hidden="true">{"✕"}</span> }
            >
                <span aria-hidden="true">{"…"}</span>
            </Show>
            <span class="sr-only">
                {move || if is_deleting.get() { "Deleting…" } else { "Delete session" }}
            </span>
        </button>
    }
}

#[component]
fn SessionSidebarRenameForm(
    rename_draft: RwSignal<String>,
    #[prop(into)] is_saving_rename: Signal<bool>,
    #[prop(into)] save_disabled: Signal<bool>,
    on_commit_rename: Callback<()>,
    on_cancel_rename: Callback<()>,
) -> impl IntoView {
    let rename_form = NodeRef::<leptos::html::Div>::new();
    let rename_form_for_focusout = rename_form;

    view! {
        <div
            class="session-sidebar__rename-form"
            node_ref=rename_form
            on:focusout=move |ev: web_sys::FocusEvent| {
                let Some(container) = rename_form_for_focusout.get() else {
                    return;
                };
                let container = container.unchecked_into::<web_sys::Node>();
                if focus_event_leaves_node(&ev, &container) {
                    on_commit_rename.run(());
                }
            }
        >
            <SessionSidebarRenameInput
                rename_draft=rename_draft
                is_saving_rename=is_saving_rename
                on_commit_rename=on_commit_rename
                on_cancel_rename=on_cancel_rename
            />
            <SessionSidebarRenameButtons
                is_saving_rename=is_saving_rename
                save_disabled=save_disabled
                on_commit_rename=on_commit_rename
                on_cancel_rename=on_cancel_rename
            />
        </div>
    }
}

fn focus_event_leaves_node(ev: &web_sys::FocusEvent, container: &web_sys::Node) -> bool {
    let Some(related_target) = ev.related_target() else {
        return true;
    };
    let Ok(related_node) = related_target.dyn_into::<web_sys::Node>() else {
        return true;
    };
    !container.contains(Some(&related_node))
}

#[component]
fn SessionSidebarRenameInput(
    rename_draft: RwSignal<String>,
    #[prop(into)] is_saving_rename: Signal<bool>,
    on_commit_rename: Callback<()>,
    on_cancel_rename: Callback<()>,
) -> impl IntoView {
    view! {
        <input
            class="session-sidebar__rename-input"
            type="text"
            autofocus=true
            maxlength="500"
            prop:value=move || rename_draft.get()
            prop:disabled=move || is_saving_rename.get()
            on:input=move |ev| {
                rename_draft.set(event_target_value(&ev));
            }
            on:keydown=move |ev: web_sys::KeyboardEvent| match ev.key().as_str() {
                "Enter" => {
                    ev.prevent_default();
                    on_commit_rename.run(());
                }
                "Escape" => {
                    ev.prevent_default();
                    on_cancel_rename.run(());
                }
                _ => {}
            }
        />
    }
}

#[component]
fn SessionSidebarRenameButtons(
    #[prop(into)] is_saving_rename: Signal<bool>,
    #[prop(into)] save_disabled: Signal<bool>,
    on_commit_rename: Callback<()>,
    on_cancel_rename: Callback<()>,
) -> impl IntoView {
    view! {
        <button
            type="button"
            class="session-sidebar__action-btn"
            on:click=move |_| on_commit_rename.run(())
            prop:disabled=move || save_disabled.get()
        >
            <Show
                when=move || is_saving_rename.get()
                fallback=|| view! { <span aria-hidden="true">{"✓"}</span> }
            >
                <span aria-hidden="true">{"…"}</span>
            </Show>
            <span class="sr-only">"Save title"</span>
        </button>
        <button
            type="button"
            class="session-sidebar__action-btn"
            on:click=move |_| on_cancel_rename.run(())
            prop:disabled=move || is_saving_rename.get()
        >
            <span aria-hidden="true">{"✕"}</span>
            <span class="sr-only">"Cancel rename"</span>
        </button>
    }
}

#[component]
fn SessionDock(
    composer: SessionComposerSignals,
    callbacks: SessionViewCallbacks,
    draft: RwSignal<String>,
) -> impl IntoView {
    let slash_signals = composer_slash_signals(composer);

    view! {
        <div class="chat-dock">
            <Composer
                disabled=composer.disabled
                status_text=composer.status
                draft=draft
                on_submit=callbacks.submit
                show_cancel=composer.cancel_visible
                cancel_disabled=composer.cancel_busy
                on_cancel=callbacks.cancel
                slash_signals=slash_signals
                slash_callbacks=callbacks.slash
            />
        </div>
    }
}

fn composer_slash_signals(composer: SessionComposerSignals) -> ComposerSlashSignals {
    ComposerSlashSignals {
        visible: composer.slash_palette_visible,
        candidates: composer.slash_candidates,
        selected_index: composer.slash_selected_index,
        apply_on_enter: composer.slash_apply_on_enter,
    }
}

fn session_permission_callbacks(
    session_id: String,
    signals: SessionSignals,
) -> (Callback<String>, Callback<String>, Callback<()>) {
    (
        permission_resolution_callback(session_id.clone(), PermissionDecision::Approve, signals),
        permission_resolution_callback(session_id.clone(), PermissionDecision::Deny, signals),
        cancel_turn_callback(session_id, signals),
    )
}

fn spawn_session_bootstrap(session_id: String, signals: SessionSignals) {
    leptos::task::spawn_local(async move {
        match api::load_session(&session_id).await {
            Ok(session) => {
                let is_closed = session.status == SessionStatus::Closed;
                apply_loaded_session(session, signals);
                refresh_session_list(signals).await;
                if !is_closed {
                    spawn_session_stream(session_id.clone(), signals);
                }
            }
            Err(api::SessionLoadError::ResumeUnavailable(message)) => {
                record_session_bootstrap_failure(message, SessionLifecycle::Unavailable, signals);
                refresh_session_list(signals).await;
            }
            Err(api::SessionLoadError::Other(message)) => {
                record_session_bootstrap_failure(message, SessionLifecycle::Error, signals);
                refresh_session_list(signals).await;
            }
        }
    });
}

fn spawn_session_stream(session_id: String, signals: SessionSignals) {
    stop_live_stream(signals);
    let (abort_handle, abort_registration) = AbortHandle::new_pair();
    signals.stream_abort.set(Some(abort_handle));
    leptos::task::spawn_local(async move {
        let _ = Abortable::new(subscribe_sse(&session_id, signals), abort_registration).await;
        close_live_stream(signals);
        signals.stream_abort.set(None);
    });
}

async fn refresh_session_list(signals: SessionSignals) {
    signals.list.error.set(None);

    match api::list_sessions().await {
        Ok(sessions) => {
            signals.list.items.set(sessions);
            signals.list.loaded.set(true);
        }
        Err(message) => {
            signals.list.loaded.set(true);
            signals.list.error.set(Some(message));
        }
    }
}

fn bind_slash_completion(signals: SessionSignals) {
    Effect::new(move |_| {
        let draft = signals.draft.get();
        let candidates = local_slash_candidates(&draft);
        if candidates.is_empty() {
            dismiss_slash_palette(signals);
        } else {
            signals.slash.candidates.set(candidates);
            signals.slash.selected_index.set(0);
        }
    });
}

fn slash_palette_callbacks(signals: SessionSignals) -> ComposerSlashCallbacks {
    ComposerSlashCallbacks {
        select_next: Callback::new(move |()| {
            let next_index = cycle_slash_selection(
                signals.slash.candidates.get_untracked().len(),
                signals.slash.selected_index.get_untracked(),
                true,
            );
            signals.slash.selected_index.set(next_index);
        }),
        select_previous: Callback::new(move |()| {
            let next_index = cycle_slash_selection(
                signals.slash.candidates.get_untracked().len(),
                signals.slash.selected_index.get_untracked(),
                false,
            );
            signals.slash.selected_index.set(next_index);
        }),
        apply_selected: Callback::new(move |()| apply_selected_slash_candidate(signals)),
        apply_index: Callback::new(move |index: usize| apply_slash_candidate_at(signals, index)),
        dismiss: Callback::new(move |()| dismiss_slash_palette(signals)),
    }
}

fn apply_selected_slash_candidate(signals: SessionSignals) {
    let index = signals.slash.selected_index.get_untracked();
    apply_slash_candidate_at(signals, index);
}

fn apply_slash_candidate_at(signals: SessionSignals, index: usize) {
    let Some(candidate) = signals.slash.candidates.get_untracked().get(index).cloned() else {
        return;
    };
    let Some(next_draft) = apply_slash_completion(&signals.draft.get_untracked(), &candidate)
    else {
        return;
    };
    signals.draft.set(next_draft);
    signals.slash.selected_index.set(index);
}

fn dismiss_slash_palette(signals: SessionSignals) {
    signals.slash.candidates.set(Vec::new());
    signals.slash.selected_index.set(0);
}

async fn subscribe_sse(session_id: &str, signals: SessionSignals) {
    let (event_source, mut rx) = match api::open_session_event_stream(session_id) {
        Ok(stream) => stream,
        Err(message) => {
            signals.connection_error.set(Some(message));
            return;
        }
    };
    signals.event_source.set(Some(event_source.clone()));

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
                    signals.event_source.set(None);
                    return;
                }
            }
            api::SseItem::Disconnected => {
                if matches!(
                    signals.session_status.get_untracked(),
                    SessionLifecycle::Closed
                ) {
                    event_source.close();
                    signals.event_source.set(None);
                    return;
                }
                signals.connection_error.set(Some(
                    "Event stream disconnected; reconnecting...".to_string(),
                ));
            }
            api::SseItem::ParseError(message) => {
                signals.connection_error.set(Some(message));
                event_source.close();
                signals.event_source.set(None);
                return;
            }
        }
    }

    event_source.close();
    signals.event_source.set(None);
}

fn handle_slash_submit(prompt: &str, signals: SessionSignals) {
    match parse_browser_slash_action(prompt) {
        Ok(action) => {
            signals.action_error.set(None);
            signals.draft.set(String::new());
            dismiss_slash_palette(signals);
            run_browser_slash_action(action, signals);
        }
        Err(message) => {
            push_tool_activity_entry(
                signals,
                next_tool_activity_id(signals, "slash"),
                "Slash command",
                message,
                Vec::new(),
            );
        }
    }
}

fn run_browser_slash_action(action: BrowserSlashAction, signals: SessionSignals) {
    match action {
        BrowserSlashAction::Help => {
            let commands = local_browser_commands();
            push_tool_activity_entry(
                signals,
                next_tool_activity_id(signals, "help"),
                "Available slash commands",
                if commands.is_empty() {
                    "No browser slash commands are available.".to_string()
                } else {
                    "Use the composer for `/help` and the on-screen controls for cancel or permission actions.".to_string()
                },
                commands,
            );
        }
    }
}

fn session_submit_callback(session_id: String, signals: SessionSignals) -> Callback<String> {
    Callback::new(move |prompt: String| {
        let session_id = session_id.clone();
        if prompt.starts_with('/') {
            handle_slash_submit(&prompt, signals);
            return;
        }

        signals.turn_state.set(TurnState::Submitting);
        signals.action_error.set(None);
        dismiss_slash_palette(signals);
        leptos::task::spawn_local(async move {
            match api::send_message(&session_id, &prompt).await {
                Ok(()) => {
                    clear_prepared_session_id();
                    clear_draft(&session_id);
                    signals.draft.set(String::new());
                    signals.turn_state.set(TurnState::AwaitingReply);
                    refresh_session_list(signals).await;
                }
                Err(message) => {
                    signals.action_error.set(Some(message));
                    signals.turn_state.set(TurnState::Idle);
                }
            }
        });
    })
}

fn spawn_home_redirect(error: RwSignal<Option<String>>, preparing: RwSignal<bool>) {
    leptos::task::spawn_local(async move {
        match resolve_home_session_id().await {
            Ok(session_id) => {
                if let Err(message) = navigate_to(&app_session_path(&session_id)) {
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
    push_tool_activity_entry(
        signals,
        next_tool_activity_id(signals, "connection"),
        "Connection",
        signals.connection_error.get_untracked().unwrap_or_default(),
        Vec::new(),
    );
    signals.session_status.set(session_lifecycle);
    signals.turn_state.set(TurnState::Idle);
}

fn permission_resolution_callback(
    session_id: String,
    decision: PermissionDecision,
    signals: SessionSignals,
) -> Callback<String> {
    Callback::new(move |request_id: String| {
        let session_id = session_id.clone();
        let request_id_for_state = request_id.clone();
        let request_id_for_api = request_id.clone();
        let decision = decision.clone();
        let request_decision = decision.clone();
        signals.pending_action_busy.set(true);
        signals.action_error.set(None);
        leptos::task::spawn_local(async move {
            match api::resolve_permission(&session_id, &request_id_for_api, request_decision).await
            {
                Ok(_) => {
                    signals.pending_permissions.update(|current_permissions| {
                        current_permissions.retain(|current_permission| {
                            current_permission.request_id.as_str() != request_id_for_state.as_str()
                        });
                    });
                    signals.turn_state.set(match decision {
                        PermissionDecision::Approve => TurnState::AwaitingReply,
                        PermissionDecision::Deny => TurnState::Idle,
                    });
                    push_tool_activity_entry(
                        signals,
                        next_tool_activity_id(signals, "permission"),
                        "Permission resolved",
                        format!(
                            "{} {}.",
                            request_id_for_state,
                            if decision == PermissionDecision::Approve {
                                "approved"
                            } else {
                                "denied"
                            }
                        ),
                        Vec::new(),
                    );
                    refresh_session_list(signals).await;
                }
                Err(message) => {
                    signals.action_error.set(Some(message));
                }
            }
            signals.pending_action_busy.set(false);
        });
    })
}

fn cancel_turn_callback(session_id: String, signals: SessionSignals) -> Callback<()> {
    Callback::new(move |()| {
        let session_id = session_id.clone();
        let previous_turn_state = signals.turn_state.get_untracked();
        signals.pending_action_busy.set(true);
        signals.turn_state.set(TurnState::Cancelling);
        signals.action_error.set(None);
        leptos::task::spawn_local(async move {
            match api::cancel_turn(&session_id).await {
                Ok(cancelled) if cancelled.cancelled => {
                    signals.pending_permissions.set(Vec::new());
                    signals.turn_state.set(TurnState::Idle);
                    push_tool_activity_entry(
                        signals,
                        next_tool_activity_id(signals, "cancel"),
                        "Cancel turn",
                        "Cancel requested for the running turn.".to_string(),
                        Vec::new(),
                    );
                    refresh_session_list(signals).await;
                }
                Ok(_) => {
                    signals
                        .action_error
                        .set(Some("No running turn is active.".to_string()));
                    if signals.turn_state.get_untracked() == TurnState::Cancelling {
                        signals.turn_state.set(previous_turn_state);
                    }
                }
                Err(message) => {
                    signals.action_error.set(Some(message));
                    if signals.turn_state.get_untracked() == TurnState::Cancelling {
                        signals.turn_state.set(previous_turn_state);
                    }
                }
            }
            signals.pending_action_busy.set(false);
        });
    })
}

fn rename_session_callback(signals: SessionSignals) -> Callback<(String, String)> {
    Callback::new(move |(session_id, new_title): (String, String)| {
        let new_title = new_title.trim().to_string();
        if new_title.is_empty() {
            signals.list.rename_draft.set(String::new());
            signals.list.renaming_id.set(None);
            return;
        }
        signals.list.error.set(None);
        signals.list.saving_rename_id.set(Some(session_id.clone()));
        leptos::task::spawn_local(async move {
            match api::rename_session(&session_id, &new_title).await {
                Ok(session) => {
                    signals.list.items.update(|list| {
                        rename_session_in_list(list, &session_id, session.title);
                    });
                    signals.list.rename_draft.set(String::new());
                    signals.list.renaming_id.set(None);
                }
                Err(message) => {
                    signals.list.error.set(Some(message));
                    signals.list.rename_draft.set(new_title.clone());
                    signals.list.renaming_id.set(Some(session_id.clone()));
                }
            }
            signals.list.saving_rename_id.set(None);
        });
    })
}

fn delete_session_callback(
    current_session_id: String,
    signals: SessionSignals,
) -> Callback<String> {
    Callback::new(move |session_id: String| {
        if delete_session_is_blocked(&session_id, &current_session_id, signals) {
            return;
        }

        signals.list.deleting_id.set(Some(session_id.clone()));
        signals.list.error.set(None);
        let is_deleting_current = session_id == current_session_id;

        leptos::task::spawn_local(async move {
            match api::delete_session(&session_id).await {
                Ok(_) => {
                    clear_prepared_session_id_if_matches(&session_id);
                    clear_draft(&session_id);
                    signals
                        .list
                        .items
                        .update(|list| remove_session_from_list(list, &session_id));
                    if is_deleting_current {
                        finish_current_session_delete(signals);
                    } else {
                        finish_other_session_delete(signals).await;
                    }
                }
                Err(message) => handle_delete_session_error(message, signals),
            }
        });
    })
}

fn delete_session_is_blocked(
    session_id: &str,
    current_session_id: &str,
    signals: SessionSignals,
) -> bool {
    signals.list.deleting_id.get_untracked().is_some()
        || (session_id == current_session_id
            && session_action_busy(
                signals.turn_state.get_untracked(),
                signals.pending_action_busy.get_untracked(),
                false,
            ))
}

fn finish_current_session_delete(signals: SessionSignals) {
    let next_dest = next_session_destination(&signals.list.items.get_untracked());

    match navigate_to(&next_dest) {
        Ok(()) => stop_live_stream(signals),
        Err(message) => handle_current_session_delete_navigation_error(message, signals),
    }
}

async fn finish_other_session_delete(signals: SessionSignals) {
    refresh_session_list(signals).await;
    signals.list.deleting_id.set(None);
}

fn handle_current_session_delete_navigation_error(message: String, signals: SessionSignals) {
    stop_live_stream(signals);
    signals.pending_permissions.set(Vec::new());
    signals.turn_state.set(TurnState::Idle);
    signals.session_status.set(SessionLifecycle::Unavailable);
    signals.list.error.set(Some(message));
    signals.list.deleting_id.set(None);
}

fn handle_delete_session_error(message: String, signals: SessionSignals) {
    signals.list.error.set(Some(message));
    signals.list.deleting_id.set(None);
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
        StreamEventPayload::SessionClosed { session_id, reason } => {
            apply_session_closed(sequence, session_id, reason, signals)
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
    push_tool_activity_entry(
        signals,
        format!("permission-{request_id}"),
        "Permission required",
        summary,
        Vec::new(),
    );
}

fn apply_session_closed(
    sequence: u64,
    session_id: String,
    reason: String,
    signals: SessionSignals,
) {
    signals.session_status.set(SessionLifecycle::Closed);
    signals.turn_state.set(TurnState::Idle);
    signals.pending_permissions.set(Vec::new());
    signals.pending_action_busy.set(false);
    signals
        .list
        .items
        .update(|sessions| mark_session_closed(sessions, &session_id));
    push_status_entry(
        signals.entries,
        sequence,
        session_end_message(Some(&reason)),
    );
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
    let session_status = session_status_label(status);
    let mut entries = messages
        .into_iter()
        .map(message_to_entry)
        .collect::<Vec<_>>();
    if matches!(session_status, SessionLifecycle::Closed) {
        push_bootstrap_closed_status_entry(&mut entries);
    }

    SessionBootstrap {
        entries,
        pending_permissions: pending_permissions_to_items(pending_permissions),
        session_status,
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

fn push_activity_entry(entries: RwSignal<Vec<TranscriptEntry>>, id: String, text: String) {
    if text.trim().is_empty() {
        return;
    }

    entries.update(|current_entries| {
        if current_entries.iter().any(|entry| entry.id == id) {
            return;
        }

        current_entries.push(TranscriptEntry {
            id,
            role: EntryRole::Status,
            text,
        });
    });
}

fn push_bootstrap_closed_status_entry(entries: &mut Vec<TranscriptEntry>) {
    if entries.iter().any(|entry| {
        matches!(entry.role, EntryRole::Status) && entry.text == CLOSED_SESSION_MESSAGE
    }) {
        return;
    }

    entries.push(TranscriptEntry {
        id: "status-session-ended".to_string(),
        role: EntryRole::Status,
        text: CLOSED_SESSION_MESSAGE.to_string(),
    });
}

fn session_end_message(reason: Option<&str>) -> String {
    let Some(reason) = reason.map(str::trim) else {
        return CLOSED_SESSION_MESSAGE.to_string();
    };
    if reason.is_empty() || reason == "closed by user" {
        CLOSED_SESSION_MESSAGE.to_string()
    } else {
        reason.to_string()
    }
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
    current_session_deleting: Signal<bool>,
) -> Signal<bool> {
    Signal::derive(move || {
        session_composer_disabled(
            session_status.get(),
            turn_state.get(),
            current_session_deleting.get(),
        )
    })
}

fn session_composer_status_signal(
    turn_state: RwSignal<TurnState>,
    session_status: RwSignal<SessionLifecycle>,
    current_session_deleting: Signal<bool>,
) -> Signal<String> {
    Signal::derive(move || {
        session_composer_status_message(
            session_status.get(),
            turn_state.get(),
            current_session_deleting.get(),
        )
    })
}

fn session_composer_cancel_visible_signal(
    turn_state: RwSignal<TurnState>,
    pending_permissions: RwSignal<Vec<PendingPermission>>,
    current_session_deleting: Signal<bool>,
) -> Signal<bool> {
    Signal::derive(move || {
        session_composer_cancel_visible(
            turn_state.get(),
            !pending_permissions.get().is_empty(),
            current_session_deleting.get(),
        )
    })
}

fn session_composer_cancel_busy_signal(
    turn_state: RwSignal<TurnState>,
    pending_action_busy: RwSignal<bool>,
    current_session_deleting: Signal<bool>,
) -> Signal<bool> {
    Signal::derive(move || {
        pending_action_busy.get()
            || current_session_deleting.get()
            || matches!(turn_state.get(), TurnState::Cancelling)
    })
}

fn next_tool_activity_id(signals: SessionSignals, prefix: &str) -> String {
    let next = signals.tool_activity_serial.get_untracked() + 1;
    signals.tool_activity_serial.set(next);
    format!("{prefix}-{next}")
}

fn push_tool_activity_entry(
    signals: SessionSignals,
    id: String,
    title: impl Into<String>,
    detail: impl Into<String>,
    commands: Vec<CompletionCandidate>,
) {
    let title = title.into();
    let detail = detail.into();
    push_activity_entry(
        signals.entries,
        format!("activity-{id}"),
        tool_activity_text(&title, &detail, &commands),
    );
}

fn tool_activity_text(title: &str, detail: &str, commands: &[CompletionCandidate]) -> String {
    let mut lines = Vec::new();

    let title = title.trim();
    if !title.is_empty() {
        lines.push(title.to_string());
    }

    let detail = detail.trim();
    if !detail.is_empty() {
        lines.push(detail.to_string());
    }

    if !commands.is_empty() {
        lines.push("Commands:".to_string());
        lines.extend(commands.iter().map(format_tool_activity_command));
    }

    lines.join("\n")
}

fn format_tool_activity_command(command: &CompletionCandidate) -> String {
    let detail = command.detail.trim();
    if detail.is_empty() {
        format!("- {}", command.label)
    } else {
        format!("- {} — {}", command.label, detail)
    }
}

fn connection_badge_state(
    session_status: SessionLifecycle,
    has_connection_error: bool,
) -> StatusBadge {
    match session_status {
        SessionLifecycle::Loading => StatusBadge {
            label: "Connection",
            value: "connecting",
            tone: BadgeTone::Neutral,
        },
        SessionLifecycle::Active if has_connection_error => StatusBadge {
            label: "Connection",
            value: "reconnecting",
            tone: BadgeTone::Warning,
        },
        SessionLifecycle::Active => StatusBadge {
            label: "Connection",
            value: "live",
            tone: BadgeTone::Success,
        },
        SessionLifecycle::Closed => StatusBadge {
            label: "Connection",
            value: "ended",
            tone: BadgeTone::Neutral,
        },
        SessionLifecycle::Unavailable | SessionLifecycle::Error => StatusBadge {
            label: "Connection",
            value: "unavailable",
            tone: BadgeTone::Danger,
        },
    }
}

fn worker_badge_state(
    session_status: SessionLifecycle,
    turn_state: TurnState,
    has_pending_permissions: bool,
) -> StatusBadge {
    match session_status {
        SessionLifecycle::Loading => StatusBadge {
            label: "Worker",
            value: "starting",
            tone: BadgeTone::Neutral,
        },
        SessionLifecycle::Unavailable | SessionLifecycle::Error => StatusBadge {
            label: "Worker",
            value: "unavailable",
            tone: BadgeTone::Danger,
        },
        SessionLifecycle::Closed => StatusBadge {
            label: "Worker",
            value: "stopped",
            tone: BadgeTone::Neutral,
        },
        SessionLifecycle::Active if has_pending_permissions => StatusBadge {
            label: "Worker",
            value: "permission",
            tone: BadgeTone::Warning,
        },
        SessionLifecycle::Active => match turn_state {
            TurnState::Submitting | TurnState::AwaitingReply => StatusBadge {
                label: "Worker",
                value: "running",
                tone: BadgeTone::Success,
            },
            TurnState::Cancelling => StatusBadge {
                label: "Worker",
                value: "cancelling",
                tone: BadgeTone::Warning,
            },
            TurnState::AwaitingPermission => StatusBadge {
                label: "Worker",
                value: "permission",
                tone: BadgeTone::Warning,
            },
            TurnState::Idle => StatusBadge {
                label: "Worker",
                value: "idle",
                tone: BadgeTone::Neutral,
            },
        },
    }
}

fn status_badge_class(badge: StatusBadge) -> &'static str {
    match badge.tone {
        BadgeTone::Neutral => "status-badge status-badge--neutral",
        BadgeTone::Success => "status-badge status-badge--success",
        BadgeTone::Warning => "status-badge status-badge--warning",
        BadgeTone::Danger => "status-badge status-badge--danger",
    }
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

    route_from_pathname(&pathname)
}

fn route_from_pathname(pathname: &str) -> AppRoute {
    if pathname == "/app" || pathname == "/app/" {
        return AppRoute::Home;
    }

    pathname
        .strip_prefix("/app/sessions/")
        .filter(|session_id| !session_id.is_empty())
        .and_then(api::decode_component)
        .map(AppRoute::Session)
        .unwrap_or(AppRoute::NotFound)
}

fn app_session_path(session_id: &str) -> String {
    format!("/app/sessions/{}", api::encode_component(session_id))
}

fn default_sidebar_open() -> bool {
    web_sys::window()
        .and_then(|window| window.inner_width().ok())
        .and_then(|width| width.as_f64())
        .map(|width| width >= 960.0)
        .unwrap_or(true)
}

fn session_layout_class(sidebar_open: bool) -> &'static str {
    if sidebar_open {
        "session-layout session-layout--sidebar-open"
    } else {
        "session-layout"
    }
}

fn session_sidebar_class(sidebar_open: bool) -> &'static str {
    if sidebar_open {
        "session-sidebar session-sidebar--open"
    } else {
        "session-sidebar"
    }
}

fn session_sidebar_item_class(is_current: bool, is_closed: bool) -> &'static str {
    match (is_current, is_closed) {
        (true, true) => {
            "session-sidebar__item session-sidebar__item--current session-sidebar__item--closed"
        }
        (true, false) => "session-sidebar__item session-sidebar__item--current",
        (false, true) => "session-sidebar__item session-sidebar__item--closed",
        (false, false) => "session-sidebar__item",
    }
}

fn session_sidebar_empty_message(has_error: bool) -> &'static str {
    if has_error {
        "Unable to load sessions right now."
    } else {
        "No sessions yet. Start a new one."
    }
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

fn clear_prepared_session_id_if_matches(session_id: &str) {
    if prepared_session_id().as_deref() == Some(session_id) {
        clear_prepared_session_id();
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

fn draft_storage_key(session_id: &str) -> String {
    format!("{DRAFT_STORAGE_KEY_PREFIX}{session_id}")
}

fn load_draft(session_id: &str) -> String {
    session_storage()
        .and_then(|storage| {
            storage
                .get_item(&draft_storage_key(session_id))
                .ok()
                .flatten()
        })
        .unwrap_or_default()
}

fn save_draft(session_id: &str, text: &str) {
    if let Some(storage) = session_storage() {
        if text.is_empty() {
            let _ = storage.remove_item(&draft_storage_key(session_id));
        } else {
            let _ = storage.set_item(&draft_storage_key(session_id), text);
        }
    }
}

fn clear_draft(session_id: &str) {
    if let Some(storage) = session_storage() {
        let _ = storage.remove_item(&draft_storage_key(session_id));
    }
}

fn sidebar_sessions(sessions: &[SessionListItem], current_session_id: &str) -> Vec<SidebarSession> {
    sessions
        .iter()
        .map(|session| SidebarSession {
            href: app_session_path(&session.id),
            title: if session.title.is_empty() {
                "New chat".to_string()
            } else {
                session.title.clone()
            },
            id: session.id.clone(),
            is_current: session.id == current_session_id,
            is_closed: matches!(session.status, SessionStatus::Closed),
        })
        .collect()
}

fn mark_session_closed(sessions: &mut [SessionListItem], session_id: &str) {
    if let Some(session) = sessions.iter_mut().find(|session| session.id == session_id) {
        session.status = SessionStatus::Closed;
    }
}

fn remove_session_from_list(sessions: &mut Vec<SessionListItem>, session_id: &str) {
    sessions.retain(|session| session.id != session_id);
}

fn next_session_destination(sessions: &[SessionListItem]) -> String {
    sessions
        .first()
        .map(|session| app_session_path(&session.id))
        .unwrap_or_else(|| "/app/".to_string())
}

fn rename_session_in_list(sessions: &mut [SessionListItem], session_id: &str, title: String) {
    if let Some(session) = sessions.iter_mut().find(|session| session.id == session_id) {
        session.title = title;
    }
}

fn stop_live_stream(signals: SessionSignals) {
    if let Some(abort_handle) = signals.stream_abort.get_untracked() {
        abort_handle.abort();
        signals.stream_abort.set(None);
    }
    close_live_stream(signals);
}

fn close_live_stream(signals: SessionSignals) {
    if let Some(event_source) = signals.event_source.get_untracked() {
        event_source.close();
        signals.event_source.set(None);
    }
}

fn session_action_busy(
    turn_state: TurnState,
    pending_action_busy: bool,
    action_in_progress: bool,
) -> bool {
    pending_action_busy || action_in_progress || turn_state != TurnState::Idle
}

fn session_composer_disabled(
    session_status: SessionLifecycle,
    turn_state: TurnState,
    current_session_deleting: bool,
) -> bool {
    current_session_deleting
        || session_status != SessionLifecycle::Active
        || turn_state != TurnState::Idle
}

fn session_composer_status_message(
    session_status: SessionLifecycle,
    turn_state: TurnState,
    current_session_deleting: bool,
) -> String {
    if current_session_deleting {
        return "Deleting session...".to_string();
    }
    match turn_state {
        TurnState::Submitting | TurnState::AwaitingReply => "Waiting for response...".to_string(),
        TurnState::AwaitingPermission => {
            "Resolve the pending request before sending another message.".to_string()
        }
        TurnState::Cancelling => "Cancelling...".to_string(),
        TurnState::Idle => match session_status {
            SessionLifecycle::Active => String::new(),
            SessionLifecycle::Closed => "This conversation has ended.".to_string(),
            SessionLifecycle::Loading => "Connecting...".to_string(),
            SessionLifecycle::Unavailable | SessionLifecycle::Error => {
                "Session unavailable. Start a fresh chat.".to_string()
            }
        },
    }
}

fn session_composer_cancel_visible(
    turn_state: TurnState,
    has_pending_permissions: bool,
    current_session_deleting: bool,
) -> bool {
    !current_session_deleting
        && !has_pending_permissions
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
        AppRoute, BadgeTone, EntryRole, PendingPermission, SessionLifecycle, SidebarSession,
        TurnState, app_session_path, connection_badge_state, mark_session_closed,
        next_session_destination, remove_session_from_list, rename_session_in_list,
        route_from_pathname, session_action_busy, session_bootstrap_from_snapshot,
        session_composer_cancel_visible, session_composer_disabled,
        session_composer_status_message, should_release_turn_state, sidebar_sessions,
        turn_state_for_snapshot, worker_badge_state,
    };
    use acp_contracts::{
        ConversationMessage, MessageRole, PermissionRequest, SessionListItem, SessionResponse,
        SessionSnapshot, SessionStatus,
    };
    use chrono::{TimeZone, Utc};

    fn sample_session_bootstrap_response() -> SessionResponse {
        SessionResponse {
            session: SessionSnapshot {
                id: "s_123".to_string(),
                title: "My test session".to_string(),
                status: SessionStatus::Closed,
                latest_sequence: 8,
                messages: vec![
                    ConversationMessage {
                        id: "m_user".to_string(),
                        role: MessageRole::User,
                        text: "hello".to_string(),
                        created_at: Utc.with_ymd_and_hms(2026, 4, 17, 1, 0, 0).unwrap(),
                    },
                    ConversationMessage {
                        id: "m_assistant".to_string(),
                        role: MessageRole::Assistant,
                        text: "world".to_string(),
                        created_at: Utc.with_ymd_and_hms(2026, 4, 17, 1, 0, 1).unwrap(),
                    },
                ],
                pending_permissions: vec![PermissionRequest {
                    request_id: "req_1".to_string(),
                    summary: "read README.md".to_string(),
                }],
            },
        }
    }

    #[test]
    fn session_composer_is_disabled_while_a_reply_is_pending() {
        assert!(session_composer_disabled(
            SessionLifecycle::Active,
            TurnState::AwaitingReply,
            false,
        ));
    }

    #[test]
    fn session_composer_prompts_for_permission_resolution_before_new_messages() {
        assert_eq!(
            session_composer_status_message(
                SessionLifecycle::Active,
                TurnState::AwaitingPermission,
                false,
            ),
            "Resolve the pending request before sending another message."
        );
    }

    #[test]
    fn active_session_hides_idle_status_copy() {
        assert_eq!(
            session_composer_status_message(SessionLifecycle::Active, TurnState::Idle, false),
            ""
        );
    }

    #[test]
    fn closed_session_shows_ended_status_copy() {
        assert_eq!(
            session_composer_status_message(SessionLifecycle::Closed, TurnState::Idle, false),
            "This conversation has ended."
        );
    }

    #[test]
    fn pending_turns_show_the_cancel_action() {
        assert!(session_composer_cancel_visible(
            TurnState::AwaitingReply,
            false,
            false,
        ));
        assert!(!session_composer_cancel_visible(
            TurnState::AwaitingPermission,
            true,
            false,
        ));
    }

    #[test]
    fn deleting_session_disables_the_composer_and_updates_status_copy() {
        assert!(session_composer_disabled(
            SessionLifecycle::Active,
            TurnState::Idle,
            true,
        ));
        assert_eq!(
            session_composer_status_message(SessionLifecycle::Active, TurnState::Idle, true),
            "Deleting session..."
        );
        assert!(!session_composer_cancel_visible(
            TurnState::AwaitingReply,
            false,
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
        let bootstrap =
            session_bootstrap_from_snapshot(sample_session_bootstrap_response().session);

        assert_eq!(bootstrap.session_status, SessionLifecycle::Closed);
        assert_eq!(bootstrap.entries.len(), 3);
        assert_eq!(bootstrap.entries[0].id, "m_user");
        assert!(matches!(bootstrap.entries[0].role, EntryRole::User));
        assert_eq!(bootstrap.entries[1].id, "m_assistant");
        assert!(matches!(bootstrap.entries[1].role, EntryRole::Assistant));
        assert!(matches!(bootstrap.entries[2].role, EntryRole::Status));
        assert_eq!(bootstrap.entries[2].text, super::CLOSED_SESSION_MESSAGE);
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

    #[test]
    fn sidebar_sessions_preserve_backend_order_use_title_and_mark_current_state() {
        let sessions = vec![
            SessionListItem {
                id: "s_newest".to_string(),
                title: "Task about rust".to_string(),
                status: SessionStatus::Active,
                last_activity_at: Utc.with_ymd_and_hms(2026, 4, 17, 1, 0, 2).unwrap(),
            },
            SessionListItem {
                id: "s_closed".to_string(),
                title: "Old exploration".to_string(),
                status: SessionStatus::Closed,
                last_activity_at: Utc.with_ymd_and_hms(2026, 4, 17, 1, 0, 1).unwrap(),
            },
        ];

        assert_eq!(
            sidebar_sessions(&sessions, "s_closed"),
            vec![
                SidebarSession {
                    id: "s_newest".to_string(),
                    href: "/app/sessions/s_newest".to_string(),
                    title: "Task about rust".to_string(),
                    is_current: false,
                    is_closed: false,
                },
                SidebarSession {
                    id: "s_closed".to_string(),
                    href: "/app/sessions/s_closed".to_string(),
                    title: "Old exploration".to_string(),
                    is_current: true,
                    is_closed: true,
                },
            ]
        );
    }

    #[test]
    fn app_session_path_encodes_reserved_session_id_characters() {
        assert_eq!(app_session_path("s/1"), "/app/sessions/s%2F1");
    }

    #[test]
    fn route_from_pathname_decodes_session_id_segments() {
        assert_eq!(
            route_from_pathname("/app/sessions/s%2F1"),
            AppRoute::Session("s/1".to_string())
        );
        assert_eq!(route_from_pathname("/app/sessions/%ZZ"), AppRoute::NotFound);
    }

    #[test]
    fn sidebar_sessions_uses_new_chat_fallback_for_empty_title() {
        let sessions = vec![SessionListItem {
            id: "s_abc".to_string(),
            title: String::new(),
            status: SessionStatus::Active,
            last_activity_at: Utc.with_ymd_and_hms(2026, 4, 17, 1, 0, 0).unwrap(),
        }];

        let items = sidebar_sessions(&sessions, "s_other");
        assert_eq!(items[0].title, "New chat");
    }

    #[test]
    fn mark_session_closed_updates_only_the_target_session() {
        let mut sessions = vec![
            SessionListItem {
                id: "s_current".to_string(),
                title: "Active one".to_string(),
                status: SessionStatus::Active,
                last_activity_at: Utc.with_ymd_and_hms(2026, 4, 17, 1, 0, 2).unwrap(),
            },
            SessionListItem {
                id: "s_other".to_string(),
                title: "Other one".to_string(),
                status: SessionStatus::Active,
                last_activity_at: Utc.with_ymd_and_hms(2026, 4, 17, 1, 0, 1).unwrap(),
            },
        ];

        mark_session_closed(&mut sessions, "s_other");

        assert_eq!(sessions[0].status, SessionStatus::Active);
        assert_eq!(sessions[1].status, SessionStatus::Closed);
    }

    #[test]
    fn remove_session_from_list_drops_the_target_session() {
        let mut sessions = vec![
            SessionListItem {
                id: "s_keep".to_string(),
                title: "Keep me".to_string(),
                status: SessionStatus::Active,
                last_activity_at: Utc.with_ymd_and_hms(2026, 4, 17, 1, 0, 2).unwrap(),
            },
            SessionListItem {
                id: "s_remove".to_string(),
                title: "Remove me".to_string(),
                status: SessionStatus::Active,
                last_activity_at: Utc.with_ymd_and_hms(2026, 4, 17, 1, 0, 1).unwrap(),
            },
        ];

        remove_session_from_list(&mut sessions, "s_remove");

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "s_keep");
    }

    #[test]
    fn next_session_destination_uses_first_session_or_home() {
        let sessions = vec![SessionListItem {
            id: "s_keep".to_string(),
            title: "Keep me".to_string(),
            status: SessionStatus::Active,
            last_activity_at: Utc.with_ymd_and_hms(2026, 4, 17, 1, 0, 2).unwrap(),
        }];

        assert_eq!(next_session_destination(&sessions), "/app/sessions/s_keep");
        assert_eq!(next_session_destination(&[]), "/app/");
    }

    #[test]
    fn rename_session_in_list_updates_only_the_target_title() {
        let mut sessions = vec![
            SessionListItem {
                id: "s_a".to_string(),
                title: "Original A".to_string(),
                status: SessionStatus::Active,
                last_activity_at: Utc.with_ymd_and_hms(2026, 4, 17, 1, 0, 2).unwrap(),
            },
            SessionListItem {
                id: "s_b".to_string(),
                title: "Original B".to_string(),
                status: SessionStatus::Active,
                last_activity_at: Utc.with_ymd_and_hms(2026, 4, 17, 1, 0, 1).unwrap(),
            },
        ];

        rename_session_in_list(&mut sessions, "s_a", "Renamed A".to_string());

        assert_eq!(sessions[0].title, "Renamed A");
        assert_eq!(sessions[1].title, "Original B");
    }

    #[test]
    fn session_action_busy_blocks_inflight_actions() {
        assert!(!session_action_busy(TurnState::Idle, false, false));
        assert!(session_action_busy(TurnState::Submitting, false, false));
        assert!(session_action_busy(TurnState::Idle, true, false));
        assert!(session_action_busy(TurnState::Idle, false, true));
    }

    #[test]
    fn connection_and_worker_badges_reflect_live_and_reconnecting_states() {
        let connection = connection_badge_state(SessionLifecycle::Active, true);
        let worker = worker_badge_state(SessionLifecycle::Active, TurnState::AwaitingReply, false);

        assert_eq!(connection.value, "reconnecting");
        assert_eq!(connection.tone, BadgeTone::Warning);
        assert_eq!(worker.value, "running");
        assert_eq!(worker.tone, BadgeTone::Success);
    }
}
