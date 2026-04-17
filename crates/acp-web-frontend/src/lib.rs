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
    ConversationMessage, MessageRole, PermissionDecision, PermissionRequest, SessionListItem,
    SessionSnapshot, SessionStatus, StreamEvent, StreamEventPayload,
};
use futures_util::{
    StreamExt,
    future::{AbortHandle, Abortable},
};
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::EventSource;

use components::{Composer, ErrorBanner, PendingPermissions, Transcript};

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
    event_source: RwSignal<Option<EventSource>>,
    stream_abort: RwSignal<Option<AbortHandle>>,
    session_list: RwSignal<Vec<SessionListItem>>,
    session_list_loaded: RwSignal<bool>,
    session_list_error: RwSignal<Option<String>>,
    session_status: RwSignal<SessionLifecycle>,
    turn_state: RwSignal<TurnState>,
    pending_action_busy: RwSignal<bool>,
    deleting_session_id: RwSignal<Option<String>>,
    renaming_session_id: RwSignal<Option<String>>,
    saving_rename_session_id: RwSignal<Option<String>>,
    rename_draft: RwSignal<String>,
    draft: RwSignal<String>,
}

#[derive(Clone, Copy)]
struct SessionViewCallbacks {
    submit: Callback<String>,
    approve: Callback<String>,
    deny: Callback<String>,
    cancel: Callback<()>,
    rename_session: Callback<(String, String)>,
    delete_session: Callback<String>,
}

#[derive(Clone, Copy)]
struct SessionComposerSignals {
    disabled: Signal<bool>,
    status: Signal<String>,
    cancel_visible: Signal<bool>,
    cancel_busy: Signal<bool>,
}

#[derive(Clone, Copy)]
struct SessionShellSignals {
    sessions: Signal<Vec<SessionListItem>>,
    session_list_loaded: Signal<bool>,
    session_list_error: Signal<Option<String>>,
    deleting_session_id: Signal<Option<String>>,
    delete_disabled: Signal<bool>,
    renaming_session_id: RwSignal<Option<String>>,
    saving_rename_session_id: Signal<Option<String>>,
    rename_draft: RwSignal<String>,
}

#[derive(Clone, Copy)]
struct SessionMainSignals {
    session_status: Signal<SessionLifecycle>,
    topbar_message: Signal<Option<String>>,
    entries: Signal<Vec<TranscriptEntry>>,
    pending_permissions: Signal<Vec<PendingPermission>>,
    pending_action_busy: Signal<bool>,
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
    let current_session_deleting = {
        let session_id = session_id.clone();
        Signal::derive(move || {
            signals.deleting_session_id.get().as_deref() == Some(session_id.as_str())
        })
    };

    // Load persisted draft before bootstrap so it is in place when the
    // composer renders for the first time.
    let stored_draft = load_draft(&session_id);
    if !stored_draft.is_empty() {
        signals.draft.set(stored_draft);
    }

    // Persist draft changes to sessionStorage across navigations.
    let session_id_for_draft = session_id.clone();
    Effect::new(move |_| {
        save_draft(&session_id_for_draft, &signals.draft.get());
    });

    spawn_session_bootstrap(session_id.clone(), signals);

    let composer = SessionComposerSignals {
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
    };
    let on_submit = session_submit_callback(session_id.clone(), signals);
    let (on_approve, on_deny, on_cancel) =
        session_permission_callbacks(session_id.clone(), signals);
    let callbacks = SessionViewCallbacks {
        submit: on_submit,
        approve: on_approve,
        deny: on_deny,
        cancel: on_cancel,
        rename_session: rename_session_callback(signals),
        delete_session: delete_session_callback(session_id.clone(), signals),
    };

    session_view_content(session_id, signals, composer, callbacks, sidebar_open)
}

fn session_signals() -> SessionSignals {
    SessionSignals {
        entries: RwSignal::new(Vec::new()),
        pending_permissions: RwSignal::new(Vec::new()),
        action_error: RwSignal::new(None::<String>),
        connection_error: RwSignal::new(None::<String>),
        event_source: RwSignal::new(None::<EventSource>),
        stream_abort: RwSignal::new(None::<AbortHandle>),
        session_list: RwSignal::new(Vec::new()),
        session_list_loaded: RwSignal::new(false),
        session_list_error: RwSignal::new(None::<String>),
        session_status: RwSignal::new(SessionLifecycle::Loading),
        turn_state: RwSignal::new(TurnState::Idle),
        pending_action_busy: RwSignal::new(false),
        deleting_session_id: RwSignal::new(None::<String>),
        renaming_session_id: RwSignal::new(None::<String>),
        saving_rename_session_id: RwSignal::new(None::<String>),
        rename_draft: RwSignal::new(String::new()),
        draft: RwSignal::new(String::new()),
    }
}

fn session_view_content(
    current_session_id: String,
    signals: SessionSignals,
    composer: SessionComposerSignals,
    callbacks: SessionViewCallbacks,
    sidebar_open: RwSignal<bool>,
) -> impl IntoView {
    let entries = signals.entries;
    let pending_permissions = signals.pending_permissions;
    let pending_action_busy = signals.pending_action_busy;
    let action_error = signals.action_error;
    let connection_error = signals.connection_error;
    let session_list = signals.session_list;
    let session_list_loaded = signals.session_list_loaded;
    let session_list_error = signals.session_list_error;
    let deleting_session_id = signals.deleting_session_id;
    let combined_error = Signal::derive(move || action_error.get().or(connection_error.get()));
    let delete_disabled = Signal::derive(move || {
        session_action_busy(signals.turn_state.get(), pending_action_busy.get(), false)
    });
    let draft = signals.draft;
    let shell_signals = SessionShellSignals {
        sessions: Signal::derive(move || session_list.get()),
        session_list_loaded: Signal::derive(move || session_list_loaded.get()),
        session_list_error: Signal::derive(move || session_list_error.get()),
        deleting_session_id: Signal::derive(move || deleting_session_id.get()),
        delete_disabled,
        renaming_session_id: signals.renaming_session_id,
        saving_rename_session_id: Signal::derive(move || signals.saving_rename_session_id.get()),
        rename_draft: signals.rename_draft,
    };
    let main_signals = SessionMainSignals {
        session_status: Signal::derive(move || signals.session_status.get()),
        topbar_message: combined_error,
        entries: Signal::derive(move || entries.get()),
        pending_permissions: Signal::derive(move || pending_permissions.get()),
        pending_action_busy: Signal::derive(move || pending_action_busy.get()),
    };

    view! {
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
                session_list_loaded=shell_signals.session_list_loaded
                session_list_error=shell_signals.session_list_error
                sidebar_open=sidebar_open
                deleting_session_id=shell_signals.deleting_session_id
                delete_disabled=shell_signals.delete_disabled
                renaming_session_id=shell_signals.renaming_session_id
                saving_rename_session_id=shell_signals.saving_rename_session_id
                rename_draft=shell_signals.rename_draft
                on_rename_session=on_rename_session
                on_delete_session=on_delete_session
            />
            <SessionBackdrop sidebar_open=sidebar_open />
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
        <button
            type="button"
            class="session-layout__backdrop"
            hidden=move || !sidebar_open.get()
            on:click=move |_| sidebar_open.set(false)
        >
            <span class="sr-only">"Close session sidebar"</span>
        </button>
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
    let SessionViewCallbacks {
        submit: on_submit,
        approve: on_approve,
        deny: on_deny,
        cancel: on_cancel,
        ..
    } = callbacks;
    let session_status = main_signals.session_status;

    view! {
        <section class="session-main">
            <SessionTopBar message=main_signals.topbar_message sidebar_open=sidebar_open />
            <div class="chat-body">
                <Transcript entries=main_signals.entries />
            </div>
            <Show when=move || matches!(session_status.get(), SessionLifecycle::Closed)>
                <div class="session-ended-notice" role="status">
                    <p class="session-ended-notice__text">
                        "This conversation has ended. "
                        <a href="/app/">"Start a new chat."</a>
                    </p>
                </div>
            </Show>
            <SessionDock
                pending_permissions=main_signals.pending_permissions
                pending_action_busy=main_signals.pending_action_busy
                on_approve=on_approve
                on_deny=on_deny
                on_cancel=on_cancel
                composer_disabled=composer.disabled
                composer_status=composer.status
                draft=draft
                on_submit=on_submit
                composer_cancel_visible=composer.cancel_visible
                composer_cancel_busy=composer.cancel_busy
                composer_cancel=on_cancel
            />
        </section>
    }
}

#[component]
fn SessionTopBar(
    #[prop(into)] message: Signal<Option<String>>,
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
            </div>
            <ErrorBanner message=message />
        </div>
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
            <a class="session-sidebar__new-link" href="/app/">
                "New chat"
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
                <SessionSidebarListOrEmpty
                    session_items=session_items
                    session_list_error=session_list_error
                    deleting_session_id=deleting_session_id
                    delete_disabled=delete_disabled
                    renaming_session_id=renaming_session_id
                    saving_rename_session_id=saving_rename_session_id
                    rename_draft=rename_draft
                    on_rename_session=on_rename_session
                    on_delete_session=on_delete_session
                />
            </Show>
        </nav>
    }
}

#[component]
fn SessionSidebarListOrEmpty(
    #[prop(into)] session_items: Signal<Vec<SidebarSession>>,
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
    let session_id = item.id.clone();
    let is_current = item.is_current;
    let is_closed = item.is_closed;
    let href = item.href.clone();
    let title = item.title.clone();
    let title_for_rename_init = item.title.clone();

    let is_renaming = {
        let session_id = session_id.clone();
        Signal::derive(move || renaming_session_id.get().as_deref() == Some(session_id.as_str()))
    };
    let is_deleting = {
        let session_id = session_id.clone();
        Signal::derive(move || deleting_session_id.get().as_deref() == Some(session_id.as_str()))
    };
    let is_saving_rename = {
        let session_id = session_id.clone();
        Signal::derive(move || {
            saving_rename_session_id.get().as_deref() == Some(session_id.as_str())
        })
    };

    // Use Callback wrappers so the closures are Copy + callable multiple times
    // (the fallback view of <Show> is an Fn, so all captured closures must be Fn too).
    let on_begin_rename: Callback<web_sys::MouseEvent> = {
        let session_id = session_id.clone();
        Callback::new(move |_| {
            rename_draft.set(title_for_rename_init.clone());
            renaming_session_id.set(Some(session_id.clone()));
        })
    };
    let on_cancel_rename: Callback<web_sys::MouseEvent> = Callback::new(move |_| {
        rename_draft.set(String::new());
        renaming_session_id.set(None);
    });

    let on_commit_rename: Callback<()> = {
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
    let on_delete: Callback<web_sys::MouseEvent> = {
        let session_id = session_id.clone();
        Callback::new(move |_| on_delete_session.run(session_id.clone()))
    };

    view! {
        <li class=move || session_sidebar_item_class(is_current, is_closed)>
            <Show
                when=move || is_renaming.get()
                fallback={
                    let href = href.clone();
                    let title = title.clone();
                    move || {
                        view! {
                            <a
                                class="session-sidebar__session-link"
                                href=href.clone()
                                aria-current=if is_current { Some("page") } else { None }
                            >
                                <span class="session-sidebar__session-title">{title.clone()}</span>
                            </a>
                            <button
                                type="button"
                                class="session-sidebar__action-btn"
                                title="Rename"
                                on:click=move |ev| on_begin_rename.run(ev)
                                prop:disabled=move || {
                                    is_deleting.get() || saving_rename_session_id.get().is_some()
                                }
                            >
                                <span aria-hidden="true">{"✎"}</span>
                                <span class="sr-only">"Rename session"</span>
                            </button>
                            <button
                                type="button"
                                class="session-sidebar__action-btn session-sidebar__action-btn--danger"
                                title="Delete"
                                on:click=move |ev| on_delete.run(ev)
                                prop:disabled=move || {
                                    is_deleting.get()
                                        || deleting_session_id.get().is_some()
                                        || saving_rename_session_id.get().is_some()
                                        || (is_current && delete_disabled.get())
                                }
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
                }
            >
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
                    on:keydown=move |ev: web_sys::KeyboardEvent| {
                        match ev.key().as_str() {
                            "Enter" => { ev.prevent_default(); on_commit_rename.run(()); }
                            "Escape" => { ev.prevent_default(); renaming_session_id.set(None); }
                            _ => {}
                        }
                    }
                />
                <button
                    type="button"
                    class="session-sidebar__action-btn"
                    on:click=move |_| on_commit_rename.run(())
                    prop:disabled=move || {
                        is_saving_rename.get() || rename_draft.get().trim().is_empty()
                    }
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
                    on:click=move |ev| on_cancel_rename.run(ev)
                    prop:disabled=move || is_saving_rename.get()
                >
                    <span aria-hidden="true">{"✕"}</span>
                    <span class="sr-only">"Cancel rename"</span>
                </button>
            </Show>
        </li>
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
    signals.session_list_error.set(None);

    match api::list_sessions().await {
        Ok(sessions) => {
            signals.session_list.set(sessions);
            signals.session_list_loaded.set(true);
        }
        Err(message) => {
            signals.session_list_loaded.set(true);
            signals.session_list_error.set(Some(message));
        }
    }
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

fn session_submit_callback(session_id: String, signals: SessionSignals) -> Callback<String> {
    Callback::new(move |prompt: String| {
        let session_id = session_id.clone();
        signals.turn_state.set(TurnState::Submitting);
        signals.action_error.set(None);
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
            signals.rename_draft.set(String::new());
            signals.renaming_session_id.set(None);
            return;
        }
        signals.session_list_error.set(None);
        signals
            .saving_rename_session_id
            .set(Some(session_id.clone()));
        leptos::task::spawn_local(async move {
            match api::rename_session(&session_id, &new_title).await {
                Ok(session) => {
                    signals.session_list.update(|list| {
                        rename_session_in_list(list, &session_id, session.title);
                    });
                    signals.rename_draft.set(String::new());
                    signals.renaming_session_id.set(None);
                }
                Err(message) => {
                    signals.session_list_error.set(Some(message));
                    signals.rename_draft.set(new_title.clone());
                    signals.renaming_session_id.set(Some(session_id.clone()));
                }
            }
            signals.saving_rename_session_id.set(None);
        });
    })
}

fn delete_session_callback(
    current_session_id: String,
    signals: SessionSignals,
) -> Callback<String> {
    Callback::new(move |session_id: String| {
        let current_session_id = current_session_id.clone();
        let is_deleting_current = session_id == current_session_id;

        if signals.deleting_session_id.get_untracked().is_some() {
            return;
        }
        if is_deleting_current
            && session_action_busy(
                signals.turn_state.get_untracked(),
                signals.pending_action_busy.get_untracked(),
                false,
            )
        {
            return;
        }

        signals.deleting_session_id.set(Some(session_id.clone()));
        signals.session_list_error.set(None);

        leptos::task::spawn_local(async move {
            match api::delete_session(&session_id).await {
                Ok(_) => {
                    clear_draft(&session_id);
                    signals
                        .session_list
                        .update(|list| remove_session_from_list(list, &session_id));
                    if is_deleting_current {
                        // Navigate away: pick the next available session or home.
                        let next_dest = signals
                            .session_list
                            .get_untracked()
                            .into_iter()
                            .next()
                            .map(|s| format!("/app/sessions/{}", s.id))
                            .unwrap_or_else(|| "/app/".to_string());
                        if let Err(message) = navigate_to(&next_dest) {
                            stop_live_stream(signals);
                            signals.pending_permissions.set(Vec::new());
                            signals.turn_state.set(TurnState::Idle);
                            signals.session_status.set(SessionLifecycle::Unavailable);
                            signals.session_list_error.set(Some(message));
                            signals.deleting_session_id.set(None);
                        } else {
                            stop_live_stream(signals);
                        }
                    } else {
                        refresh_session_list(signals).await;
                        signals.deleting_session_id.set(None);
                    }
                }
                Err(message) => {
                    signals.session_list_error.set(Some(message));
                    signals.deleting_session_id.set(None);
                }
            }
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
        .session_list
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
            href: format!("/app/sessions/{}", session.id),
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
            "Resolve the request below before sending another message.".to_string()
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
        EntryRole, PendingPermission, SessionLifecycle, SidebarSession, TurnState,
        mark_session_closed, remove_session_from_list, rename_session_in_list, session_action_busy,
        session_bootstrap_from_snapshot, session_composer_cancel_visible,
        session_composer_disabled, session_composer_status_message, should_release_turn_state,
        sidebar_sessions, turn_state_for_snapshot,
    };
    use acp_contracts::{SessionListItem, SessionResponse, SessionStatus};
    use chrono::{TimeZone, Utc};

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
            "Resolve the request below before sending another message."
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
        let body = serde_json::json!({
            "session": {
                "id": "s_123",
                "title": "My test session",
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
}
