use acp_contracts::SessionListItem;
use leptos::{portal::Portal, prelude::*};
use wasm_bindgen::JsCast;

use crate::components::composer::{Composer, ComposerSlashSignals};
use crate::components::error_banner::ErrorBanner;
use crate::components::pending_permissions::ChatActivity;
use crate::components::transcript::Transcript;
use crate::domain::session::{
    PendingPermission, SessionLifecycle, SidebarSession, StatusBadge, session_sidebar_status_label,
    session_sidebar_status_pill_class, sidebar_sessions, status_badge_class,
};
use crate::domain::transcript::TranscriptEntry;
use crate::presentation::SessionSidebarAuthControls;

use super::{
    actions::{
        bind_slash_completion, delete_session_callback, rename_session_callback,
        session_permission_callbacks, session_submit_callback, slash_palette_callbacks,
        spawn_home_redirect, spawn_session_bootstrap,
    },
    state::{
        SessionComposerSignals, SessionMainSignals, SessionShellSignals,
        SessionSidebarItemCallbacks, SessionSidebarItemSignals, SessionSignals,
        SessionViewCallbacks, current_session_deleting_signal, persist_session_draft,
        restore_session_draft, session_composer_signals, session_main_signals,
        session_shell_signals, session_sidebar_item_signals, session_signals,
    },
};

/// Landing page. Prepares a fresh session and immediately redirects to the
/// live chat route so startup hints appear before the first prompt.
#[component]
pub(crate) fn HomePage() -> impl IntoView {
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

#[component]
pub(crate) fn SessionView(session_id: String) -> impl IntoView {
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
                auth_error=signals.action_error
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
    auth_error: RwSignal<Option<String>>,
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
                auth_error=auth_error
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
    auth_error: RwSignal<Option<String>>,
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
    let session_id_for_items = current_session_id.clone();
    let session_items =
        Signal::derive(move || sidebar_sessions(&sessions.get(), &session_id_for_items));

    view! {
        <aside class=move || session_sidebar_class(sidebar_open.get())>
            <SessionSidebarHeader
                current_session_id=current_session_id
                auth_error=auth_error
                sidebar_open=sidebar_open
            />
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
fn SessionSidebarHeader(
    current_session_id: String,
    auth_error: RwSignal<Option<String>>,
    sidebar_open: RwSignal<bool>,
) -> impl IntoView {
    view! {
        <div class="session-sidebar__header">
            <div class="session-sidebar__header-links">
                <a class="session-sidebar__new-link" href="/app/" aria-label="New chat">
                    <span class="session-sidebar__new-link-icon" aria-hidden="true">
                        "+"
                    </span>
                    <span class="session-sidebar__new-link-label">"New chat"</span>
                </a>
                <SessionSidebarAuthControls current_session_id=current_session_id error=auth_error />
            </div>
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
        item.id.clone(),
        item.title.clone(),
        rename_draft,
        renaming_session_id,
        item_signals.is_saving_rename,
        on_rename_session,
        on_delete_session,
    );

    session_sidebar_item_view(item, rename_draft, item_signals, callbacks)
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
    item: SidebarSession,
    rename_draft: RwSignal<String>,
    item_signals: SessionSidebarItemSignals,
    callbacks: SessionSidebarItemCallbacks,
) -> impl IntoView {
    let is_current = item.is_current;
    let is_closed = item.is_closed;
    view! {
        <li class=move || session_sidebar_item_class(is_current, is_closed)>
            <Show
                when=move || item_signals.is_renaming.get()
                fallback={
                    let href = item.href.clone();
                    let title = item.title.clone();
                    let activity_label = item.activity_label.clone();
                    move || {
                        view! {
                            <SessionSidebarItemDisplay
                                href=href.clone()
                                title=title.clone()
                                activity_label=activity_label.clone()
                                is_current=is_current
                                is_closed=is_closed
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
fn SessionSidebarItemDisplay(
    href: String,
    title: String,
    activity_label: String,
    is_current: bool,
    is_closed: bool,
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
            <span class="session-sidebar__session-copy">
                <span class="session-sidebar__session-title">{title}</span>
                <span class="session-sidebar__session-meta">
                    <span class="session-sidebar__session-activity">{activity_label}</span>
                    <span class=move || session_sidebar_status_pill_class(is_closed)>
                        {session_sidebar_status_label(is_closed)}
                    </span>
                </span>
            </span>
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
        apply_selected: composer.slash_apply_selected,
    }
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
