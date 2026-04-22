use acp_contracts_sessions::{SessionListItem, SessionStatus};
use leptos::prelude::*;
#[cfg(target_family = "wasm")]
use wasm_bindgen::JsCast;

use crate::presentation::SessionSidebarAuthControls;
use crate::routing::app_session_path;

use super::super::state::{
    SessionSidebarItemCallbacks, SessionSidebarItemSignals, session_sidebar_item_signals,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SidebarSession {
    id: String,
    href: String,
    title: String,
    activity_label: String,
    is_current: bool,
    is_closed: bool,
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
            activity_label: format!(
                "Updated {}",
                session.last_activity_at.format("%Y-%m-%d %H:%M UTC")
            ),
            id: session.id.clone(),
            is_current: session.id == current_session_id,
            is_closed: matches!(session.status, SessionStatus::Closed),
        })
        .collect()
}

fn session_sidebar_status_label(is_closed: bool) -> &'static str {
    if is_closed { "closed" } else { "active" }
}

fn session_sidebar_status_pill_class(is_closed: bool) -> &'static str {
    if is_closed {
        "session-sidebar__status-pill session-sidebar__status-pill--neutral"
    } else {
        "session-sidebar__status-pill session-sidebar__status-pill--success"
    }
}

#[component]
pub(super) fn SessionSidebar(
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

#[cfg(target_family = "wasm")]
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

#[cfg(not(target_family = "wasm"))]
#[component]
fn SessionSidebarHeader(
    current_session_id: String,
    auth_error: RwSignal<Option<String>>,
    sidebar_open: RwSignal<bool>,
) -> impl IntoView {
    let _ = sidebar_open;

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
            <button type="button" class="session-sidebar__dismiss">
                <span aria-hidden="true">{"✕"}</span>
                <span class="sr-only">"Close sidebar"</span>
            </button>
        </div>
    }
}

#[cfg(target_family = "wasm")]
#[component]
pub(super) fn SessionSidebarStatus(
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

#[cfg(not(target_family = "wasm"))]
#[component]
pub(super) fn SessionSidebarStatus(
    #[prop(into)] session_list_error: Signal<Option<String>>,
    #[prop(into)] session_items: Signal<Vec<SidebarSession>>,
) -> impl IntoView {
    let error = session_list_error.get_untracked();
    let has_items = !session_items.get_untracked().is_empty();

    if let (true, Some(message)) = (has_items, error) {
        view! { <p class="session-sidebar__status muted">{message}</p> }.into_any()
    } else {
        view! { <span hidden=true></span> }.into_any()
    }
}

struct SessionSidebarNavArgs {
    session_items: Signal<Vec<SidebarSession>>,
    session_list_loaded: Signal<bool>,
    session_list_error: Signal<Option<String>>,
    deleting_session_id: Signal<Option<String>>,
    delete_disabled: Signal<bool>,
    renaming_session_id: RwSignal<Option<String>>,
    saving_rename_session_id: Signal<Option<String>>,
    rename_draft: RwSignal<String>,
    on_rename_session: Callback<(String, String)>,
    on_delete_session: Callback<String>,
}

#[component]
pub(super) fn SessionSidebarNav(
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
    session_sidebar_nav_view(SessionSidebarNavArgs {
        session_items,
        session_list_loaded,
        session_list_error,
        deleting_session_id,
        delete_disabled,
        renaming_session_id,
        saving_rename_session_id,
        rename_draft,
        on_rename_session,
        on_delete_session,
    })
}

#[cfg(target_family = "wasm")]
fn session_sidebar_nav_view(args: SessionSidebarNavArgs) -> impl IntoView {
    let SessionSidebarNavArgs {
        session_items,
        session_list_loaded,
        session_list_error,
        deleting_session_id,
        delete_disabled,
        renaming_session_id,
        saving_rename_session_id,
        rename_draft,
        on_rename_session,
        on_delete_session,
    } = args;

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

#[cfg(not(target_family = "wasm"))]
fn session_sidebar_nav_view(args: SessionSidebarNavArgs) -> impl IntoView {
    let SessionSidebarNavArgs {
        session_items,
        session_list_loaded,
        session_list_error,
        deleting_session_id,
        delete_disabled,
        renaming_session_id,
        saving_rename_session_id,
        rename_draft,
        on_rename_session,
        on_delete_session,
    } = args;
    let loaded = session_list_loaded.get_untracked();
    let items = session_items.get_untracked();
    let has_error = session_list_error.get_untracked().is_some();

    if !loaded {
        return view! {
            <nav class="session-sidebar__nav" aria-label="Sessions">
                <p class="session-sidebar__empty muted">"Loading sessions..."</p>
            </nav>
        }
        .into_any();
    }

    if items.is_empty() {
        return view! {
            <nav class="session-sidebar__nav" aria-label="Sessions">
                <p class="session-sidebar__empty muted">
                    {session_sidebar_empty_message(has_error)}
                </p>
            </nav>
        }
        .into_any();
    }

    view! {
        <nav class="session-sidebar__nav" aria-label="Sessions">
            <SessionSidebarList
                session_items=Signal::derive(move || items.clone())
                deleting_session_id=deleting_session_id
                delete_disabled=delete_disabled
                renaming_session_id=renaming_session_id
                saving_rename_session_id=saving_rename_session_id
                rename_draft=rename_draft
                on_rename_session=on_rename_session
                on_delete_session=on_delete_session
            />
        </nav>
    }
    .into_any()
}

struct SessionSidebarListArgs {
    session_items: Signal<Vec<SidebarSession>>,
    deleting_session_id: Signal<Option<String>>,
    delete_disabled: Signal<bool>,
    renaming_session_id: RwSignal<Option<String>>,
    saving_rename_session_id: Signal<Option<String>>,
    rename_draft: RwSignal<String>,
    on_rename_session: Callback<(String, String)>,
    on_delete_session: Callback<String>,
}

#[component]
pub(super) fn SessionSidebarList(
    #[prop(into)] session_items: Signal<Vec<SidebarSession>>,
    #[prop(into)] deleting_session_id: Signal<Option<String>>,
    #[prop(into)] delete_disabled: Signal<bool>,
    renaming_session_id: RwSignal<Option<String>>,
    #[prop(into)] saving_rename_session_id: Signal<Option<String>>,
    rename_draft: RwSignal<String>,
    on_rename_session: Callback<(String, String)>,
    on_delete_session: Callback<String>,
) -> impl IntoView {
    session_sidebar_list_view(SessionSidebarListArgs {
        session_items,
        deleting_session_id,
        delete_disabled,
        renaming_session_id,
        saving_rename_session_id,
        rename_draft,
        on_rename_session,
        on_delete_session,
    })
}

#[cfg(target_family = "wasm")]
fn session_sidebar_list_view(args: SessionSidebarListArgs) -> impl IntoView {
    let SessionSidebarListArgs {
        session_items,
        deleting_session_id,
        delete_disabled,
        renaming_session_id,
        saving_rename_session_id,
        rename_draft,
        on_rename_session,
        on_delete_session,
    } = args;

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

#[cfg(not(target_family = "wasm"))]
fn session_sidebar_list_view(args: SessionSidebarListArgs) -> impl IntoView {
    let SessionSidebarListArgs {
        session_items,
        deleting_session_id,
        delete_disabled,
        renaming_session_id,
        saving_rename_session_id,
        rename_draft,
        on_rename_session,
        on_delete_session,
    } = args;
    let items = session_items.get_untracked();

    view! {
        <ul class="session-sidebar__list">
            {items
                .into_iter()
                .map(|item| {
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
                })
                .collect_view()}
        </ul>
    }
}

#[component]
pub(super) fn SessionSidebarItem(
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

pub(super) fn session_sidebar_item_callbacks(
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

#[cfg(target_family = "wasm")]
pub(super) fn session_sidebar_item_view(
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

#[cfg(not(target_family = "wasm"))]
pub(super) fn session_sidebar_item_view(
    item: SidebarSession,
    rename_draft: RwSignal<String>,
    item_signals: SessionSidebarItemSignals,
    callbacks: SessionSidebarItemCallbacks,
) -> impl IntoView {
    let is_current = item.is_current;
    let is_closed = item.is_closed;
    let item_class = session_sidebar_item_class(is_current, is_closed);

    if item_signals.is_renaming.get_untracked() {
        return view! {
            <li class=item_class>
                <SessionSidebarRenameForm
                    rename_draft=rename_draft
                    is_saving_rename=item_signals.is_saving_rename
                    save_disabled=item_signals.save_rename_disabled
                    on_commit_rename=callbacks.commit_rename
                    on_cancel_rename=callbacks.cancel_rename
                />
            </li>
        }
        .into_any();
    }

    view! {
        <li class=item_class>
            <SessionSidebarItemDisplay
                href=item.href
                title=item.title
                activity_label=item.activity_label
                is_current=is_current
                is_closed=is_closed
                is_deleting=item_signals.is_deleting
                rename_action_disabled=item_signals.rename_action_disabled
                delete_action_disabled=item_signals.delete_action_disabled
                on_begin_rename=callbacks.begin_rename
                on_delete=callbacks.delete_session
            />
        </li>
    }
    .into_any()
}

#[component]
pub(super) fn SessionSidebarItemDisplay(
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
        <SessionSidebarSessionLink
            href=href
            title=title
            activity_label=activity_label
            is_current=is_current
            is_closed=is_closed
        />
        <SessionSidebarRenameButton
            disabled=rename_action_disabled
            on_begin_rename=on_begin_rename
        />
        <SessionSidebarDeleteButton
            is_deleting=is_deleting
            disabled=delete_action_disabled
            on_delete=on_delete
        />
    }
}

#[component]
pub(super) fn SessionSidebarSessionLink(
    href: String,
    title: String,
    activity_label: String,
    is_current: bool,
    is_closed: bool,
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
    }
}

#[component]
pub(super) fn SessionSidebarRenameButton(
    #[prop(into)] disabled: Signal<bool>,
    on_begin_rename: Callback<()>,
) -> impl IntoView {
    view! {
        <button
            type="button"
            class="session-sidebar__action-btn"
            title="Rename"
            on:click=move |_| on_begin_rename.run(())
            prop:disabled=move || disabled.get()
        >
            <span aria-hidden="true">{"✎"}</span>
            <span class="sr-only">"Rename session"</span>
        </button>
    }
}

#[cfg(target_family = "wasm")]
#[component]
pub(super) fn SessionSidebarDeleteButton(
    #[prop(into)] is_deleting: Signal<bool>,
    #[prop(into)] disabled: Signal<bool>,
    on_delete: Callback<()>,
) -> impl IntoView {
    view! {
        <button
            type="button"
            class="session-sidebar__action-btn session-sidebar__action-btn--danger"
            title="Delete"
            on:click=move |_| on_delete.run(())
            prop:disabled=move || disabled.get()
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

#[cfg(not(target_family = "wasm"))]
#[component]
pub(super) fn SessionSidebarDeleteButton(
    #[prop(into)] is_deleting: Signal<bool>,
    #[prop(into)] disabled: Signal<bool>,
    on_delete: Callback<()>,
) -> impl IntoView {
    let _ = on_delete;
    let deleting = is_deleting.get_untracked();

    view! {
        <button
            type="button"
            class="session-sidebar__action-btn session-sidebar__action-btn--danger"
            title="Delete"
            prop:disabled=move || disabled.get()
        >
            <span aria-hidden="true">{if deleting { "…" } else { "✕" }}</span>
            <span class="sr-only">{sidebar_delete_sr_label(deleting)}</span>
        </button>
    }
}

#[cfg(target_family = "wasm")]
#[component]
pub(super) fn SessionSidebarRenameForm(
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

#[cfg(not(target_family = "wasm"))]
#[component]
pub(super) fn SessionSidebarRenameForm(
    rename_draft: RwSignal<String>,
    #[prop(into)] is_saving_rename: Signal<bool>,
    #[prop(into)] save_disabled: Signal<bool>,
    on_commit_rename: Callback<()>,
    on_cancel_rename: Callback<()>,
) -> impl IntoView {
    view! {
        <div class="session-sidebar__rename-form">
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

#[cfg(target_family = "wasm")]
fn focus_event_leaves_node(ev: &web_sys::FocusEvent, container: &web_sys::Node) -> bool {
    let Some(related_target) = ev.related_target() else {
        return true;
    };
    let Ok(related_node) = related_target.dyn_into::<web_sys::Node>() else {
        return true;
    };
    !container.contains(Some(&related_node))
}

#[cfg(target_family = "wasm")]
#[component]
pub(super) fn SessionSidebarRenameInput(
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

#[cfg(not(target_family = "wasm"))]
#[component]
pub(super) fn SessionSidebarRenameInput(
    rename_draft: RwSignal<String>,
    #[prop(into)] is_saving_rename: Signal<bool>,
    on_commit_rename: Callback<()>,
    on_cancel_rename: Callback<()>,
) -> impl IntoView {
    let _ = (on_commit_rename, on_cancel_rename);
    view! {
        <input
            class="session-sidebar__rename-input"
            type="text"
            maxlength="500"
            prop:value=move || rename_draft.get()
            prop:disabled=move || is_saving_rename.get()
        />
    }
}

#[component]
pub(super) fn SessionSidebarRenameButtons(
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

#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn sidebar_delete_sr_label(is_deleting: bool) -> &'static str {
    if is_deleting {
        "Deleting…"
    } else {
        "Delete session"
    }
}

pub(super) fn session_sidebar_class(sidebar_open: bool) -> &'static str {
    if sidebar_open {
        "session-sidebar session-sidebar--open"
    } else {
        "session-sidebar"
    }
}

pub(super) fn session_sidebar_item_class(is_current: bool, is_closed: bool) -> &'static str {
    match (is_current, is_closed) {
        (true, true) => {
            "session-sidebar__item session-sidebar__item--current session-sidebar__item--closed"
        }
        (true, false) => "session-sidebar__item session-sidebar__item--current",
        (false, true) => "session-sidebar__item session-sidebar__item--closed",
        (false, false) => "session-sidebar__item",
    }
}

pub(super) fn session_sidebar_empty_message(has_error: bool) -> &'static str {
    if has_error {
        "Unable to load sessions right now."
    } else {
        "No sessions yet. Start a new one."
    }
}

#[cfg(test)]
mod tests {
    use leptos::prelude::*;

    use super::{
        SidebarSession, SessionSidebarDeleteButton, SessionSidebarItem, SessionSidebarItemDisplay,
        SessionSidebarList, SessionSidebarNav, SessionSidebarRenameButton,
        SessionSidebarRenameButtons, SessionSidebarRenameForm, SessionSidebarRenameInput,
        SessionSidebarSessionLink, SessionSidebarStatus, session_sidebar_class,
        session_sidebar_empty_message, session_sidebar_item_callbacks, session_sidebar_item_class,
        session_sidebar_item_view, sidebar_delete_sr_label,
    };
    use crate::session::page::state::session_sidebar_item_signals;

    fn sample_sidebar_session() -> SidebarSession {
        SidebarSession {
            id: "s1".to_string(),
            href: "/app/sessions/s1".to_string(),
            title: "Test session".to_string(),
            activity_label: "Updated now".to_string(),
            is_current: true,
            is_closed: false,
        }
    }

    #[test]
    fn session_sidebar_class_adds_open_modifier_when_sidebar_is_open() {
        assert_eq!(
            session_sidebar_class(true),
            "session-sidebar session-sidebar--open"
        );
        assert_eq!(session_sidebar_class(false), "session-sidebar");
    }

    #[test]
    fn session_sidebar_item_class_applies_current_and_closed_modifiers() {
        let both = session_sidebar_item_class(true, true);
        assert!(both.contains("--current"));
        assert!(both.contains("--closed"));

        let current_only = session_sidebar_item_class(true, false);
        assert!(current_only.contains("--current"));
        assert!(!current_only.contains("--closed"));

        let closed_only = session_sidebar_item_class(false, true);
        assert!(!closed_only.contains("--current"));
        assert!(closed_only.contains("--closed"));

        assert_eq!(
            session_sidebar_item_class(false, false),
            "session-sidebar__item"
        );
    }

    #[test]
    fn session_sidebar_empty_message_differs_based_on_error_presence() {
        assert!(session_sidebar_empty_message(true).contains("Unable to load"));
        assert!(session_sidebar_empty_message(false).contains("No sessions yet"));
    }

    #[test]
    fn delete_labels_match_sidebar_state() {
        assert_eq!(sidebar_delete_sr_label(true), "Deleting…");
        assert_eq!(sidebar_delete_sr_label(false), "Delete session");
    }

    #[test]
    fn sidebar_item_begin_rename_sets_draft_and_renaming_id() {
        let owner = Owner::new();
        owner.with(|| {
            let rename_draft = RwSignal::new(String::new());
            let renaming_id = RwSignal::new(None::<String>);
            let is_saving = Signal::derive(|| false);
            let on_rename = Callback::new(move |_: (String, String)| {});
            let on_delete = Callback::new(move |_: String| {});

            let callbacks = session_sidebar_item_callbacks(
                "s1".to_string(),
                "My Title".to_string(),
                rename_draft,
                renaming_id,
                is_saving,
                on_rename,
                on_delete,
            );

            callbacks.begin_rename.run(());

            assert_eq!(rename_draft.get(), "My Title");
            assert_eq!(renaming_id.get(), Some("s1".to_string()));
        });
    }

    #[test]
    fn sidebar_item_cancel_rename_clears_draft_and_renaming_id() {
        let owner = Owner::new();
        owner.with(|| {
            let rename_draft = RwSignal::new("draft".to_string());
            let renaming_id = RwSignal::new(Some("s1".to_string()));
            let is_saving = Signal::derive(|| false);
            let on_rename = Callback::new(move |_: (String, String)| {});
            let on_delete = Callback::new(move |_: String| {});

            let callbacks = session_sidebar_item_callbacks(
                "s1".to_string(),
                "My Title".to_string(),
                rename_draft,
                renaming_id,
                is_saving,
                on_rename,
                on_delete,
            );

            callbacks.cancel_rename.run(());

            assert!(rename_draft.get().is_empty());
            assert!(renaming_id.get().is_none());
        });
    }

    #[test]
    fn sidebar_item_commit_rename_runs_rename_callback_when_draft_non_empty() {
        let owner = Owner::new();
        owner.with(|| {
            let rename_draft = RwSignal::new("New Name".to_string());
            let renaming_id = RwSignal::new(None::<String>);
            let is_saving = Signal::derive(|| false);
            let renamed = RwSignal::new(None::<(String, String)>);
            let on_rename = Callback::new(move |pair| renamed.set(Some(pair)));
            let on_delete = Callback::new(move |_: String| {});

            let callbacks = session_sidebar_item_callbacks(
                "s1".to_string(),
                "Old".to_string(),
                rename_draft,
                renaming_id,
                is_saving,
                on_rename,
                on_delete,
            );

            callbacks.commit_rename.run(());

            let pair = renamed.get().expect("rename callback should have fired");
            assert_eq!(pair.0, "s1");
            assert_eq!(pair.1, "New Name");
        });
    }

    #[test]
    fn sidebar_item_commit_rename_clears_renaming_id_when_draft_is_blank() {
        let owner = Owner::new();
        owner.with(|| {
            let rename_draft = RwSignal::new("  ".to_string());
            let renaming_id = RwSignal::new(Some("s1".to_string()));
            let is_saving = Signal::derive(|| false);
            let on_rename = Callback::new(move |_: (String, String)| {});
            let on_delete = Callback::new(move |_: String| {});

            let callbacks = session_sidebar_item_callbacks(
                "s1".to_string(),
                "Old".to_string(),
                rename_draft,
                renaming_id,
                is_saving,
                on_rename,
                on_delete,
            );

            callbacks.commit_rename.run(());

            assert!(renaming_id.get().is_none());
        });
    }

    #[test]
    fn sidebar_item_commit_rename_skipped_when_save_in_progress() {
        let owner = Owner::new();
        owner.with(|| {
            let rename_draft = RwSignal::new("New Name".to_string());
            let renaming_id = RwSignal::new(Some("s1".to_string()));
            let is_saving = Signal::derive(|| true);
            let renamed = RwSignal::new(None::<(String, String)>);
            let on_rename = Callback::new(move |pair| renamed.set(Some(pair)));
            let on_delete = Callback::new(move |_: String| {});

            let callbacks = session_sidebar_item_callbacks(
                "s1".to_string(),
                "Old".to_string(),
                rename_draft,
                renaming_id,
                is_saving,
                on_rename,
                on_delete,
            );

            callbacks.commit_rename.run(());

            assert!(renamed.get().is_none());
        });
    }

    #[test]
    fn sidebar_item_delete_session_forwards_the_session_id() {
        let owner = Owner::new();
        owner.with(|| {
            let rename_draft = RwSignal::new(String::new());
            let renaming_id = RwSignal::new(None::<String>);
            let is_saving = Signal::derive(|| false);
            let deleted_id = RwSignal::new(String::new());
            let on_rename = Callback::new(move |_: (String, String)| {});
            let on_delete = Callback::new(move |id: String| deleted_id.set(id));

            let callbacks = session_sidebar_item_callbacks(
                "s1".to_string(),
                "My Title".to_string(),
                rename_draft,
                renaming_id,
                is_saving,
                on_rename,
                on_delete,
            );

            callbacks.delete_session.run(());

            assert_eq!(deleted_id.get(), "s1");
        });
    }

    #[test]
    fn sidebar_display_components_build_without_panicking() {
        let owner = Owner::new();
        owner.with(|| {
            let _ = view! {
                <SessionSidebarSessionLink
                    href="/app/sessions/s1".to_string()
                    title="Test session".to_string()
                    activity_label="Updated now".to_string()
                    is_current=true
                    is_closed=false
                />
            };
            let _ = view! {
                <SessionSidebarRenameButton
                    disabled=Signal::derive(|| false)
                    on_begin_rename=Callback::new(|()| {})
                />
            };
            let _ = view! {
                <SessionSidebarDeleteButton
                    is_deleting=Signal::derive(|| true)
                    disabled=Signal::derive(|| false)
                    on_delete=Callback::new(|()| {})
                />
            };
        });
    }

    #[test]
    fn sidebar_navigation_components_build_without_panicking() {
        let owner = Owner::new();
        owner.with(|| {
            let session_items = Signal::derive(|| vec![sample_sidebar_session()]);
            let deleting_session_id = Signal::derive(|| None::<String>);
            let saving_rename_session_id = Signal::derive(|| None::<String>);
            let rename_draft = RwSignal::new("Draft".to_string());
            let renaming_session_id = RwSignal::new(None::<String>);

            let _ = view! {
                <SessionSidebarStatus
                    session_list_error=Signal::derive(|| Some("temporary".to_string()))
                    session_items=session_items
                />
            };
            let _ = view! {
                <SessionSidebarNav
                    session_items=session_items
                    session_list_loaded=Signal::derive(|| true)
                    session_list_error=Signal::derive(|| None::<String>)
                    deleting_session_id=deleting_session_id
                    delete_disabled=Signal::derive(|| false)
                    renaming_session_id=renaming_session_id
                    saving_rename_session_id=saving_rename_session_id
                    rename_draft=rename_draft
                    on_rename_session=Callback::new(|_: (String, String)| {})
                    on_delete_session=Callback::new(|_: String| {})
                />
            };
            let _ = view! {
                <SessionSidebarRenameForm
                    rename_draft=rename_draft
                    is_saving_rename=Signal::derive(|| false)
                    save_disabled=Signal::derive(|| false)
                    on_commit_rename=Callback::new(|()| {})
                    on_cancel_rename=Callback::new(|()| {})
                />
            };
        });
    }

    #[test]
    fn session_sidebar_list_and_item_build_without_panicking() {
        let owner = Owner::new();
        owner.with(|| {
            let session_items = Signal::derive(|| vec![sample_sidebar_session()]);
            let deleting_session_id = Signal::derive(|| None::<String>);
            let saving_rename_session_id = Signal::derive(|| None::<String>);
            let rename_draft = RwSignal::new(String::new());
            let renaming_session_id = RwSignal::new(None::<String>);

            let _ = view! {
                <SessionSidebarList
                    session_items=session_items
                    deleting_session_id=deleting_session_id
                    delete_disabled=Signal::derive(|| false)
                    renaming_session_id=renaming_session_id
                    saving_rename_session_id=saving_rename_session_id
                    rename_draft=rename_draft
                    on_rename_session=Callback::new(|_: (String, String)| {})
                    on_delete_session=Callback::new(|_: String| {})
                />
            };
            let _ = view! {
                <SessionSidebarItem
                    item=sample_sidebar_session()
                    deleting_session_id=deleting_session_id
                    delete_disabled=Signal::derive(|| false)
                    renaming_session_id=renaming_session_id
                    saving_rename_session_id=saving_rename_session_id
                    rename_draft=rename_draft
                    on_rename_session=Callback::new(|_: (String, String)| {})
                    on_delete_session=Callback::new(|_: String| {})
                />
            };
        });
    }

    #[test]
    fn session_sidebar_item_display_and_rename_controls_build_without_panicking() {
        let owner = Owner::new();
        owner.with(|| {
            let rename_draft = RwSignal::new("draft".to_string());

            let _ = view! {
                <SessionSidebarItemDisplay
                    href="/app/sessions/s1".to_string()
                    title="Test".to_string()
                    activity_label="Updated".to_string()
                    is_current=false
                    is_closed=false
                    is_deleting=Signal::derive(|| false)
                    rename_action_disabled=Signal::derive(|| false)
                    delete_action_disabled=Signal::derive(|| false)
                    on_begin_rename=Callback::new(|()| {})
                    on_delete=Callback::new(|()| {})
                />
            };
            let _ = view! {
                <SessionSidebarRenameInput
                    rename_draft=rename_draft
                    is_saving_rename=Signal::derive(|| false)
                    on_commit_rename=Callback::new(|()| {})
                    on_cancel_rename=Callback::new(|()| {})
                />
            };
            let _ = view! {
                <SessionSidebarRenameButtons
                    is_saving_rename=Signal::derive(|| true)
                    save_disabled=Signal::derive(|| false)
                    on_commit_rename=Callback::new(|()| {})
                    on_cancel_rename=Callback::new(|()| {})
                />
            };
        });
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn session_sidebar_nav_builds_empty_state_without_panicking() {
        let owner = Owner::new();
        owner.with(|| {
            let _ = view! {
                <SessionSidebarNav
                    session_items=Signal::derive(Vec::<SidebarSession>::new)
                    session_list_loaded=Signal::derive(|| true)
                    session_list_error=Signal::derive(|| Some("temporary".to_string()))
                    deleting_session_id=Signal::derive(|| None::<String>)
                    delete_disabled=Signal::derive(|| false)
                    renaming_session_id=RwSignal::new(None::<String>)
                    saving_rename_session_id=Signal::derive(|| None::<String>)
                    rename_draft=RwSignal::new(String::new())
                    on_rename_session=Callback::new(|_: (String, String)| {})
                    on_delete_session=Callback::new(|_: String| {})
                />
            };
        });
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn session_sidebar_item_view_builds_rename_form_when_renaming() {
        let owner = Owner::new();
        owner.with(|| {
            let item = sample_sidebar_session();
            let rename_draft = RwSignal::new("Renamed".to_string());
            let renaming_session_id = RwSignal::new(Some(item.id.clone()));
            let item_signals = session_sidebar_item_signals(
                item.id.clone(),
                item.is_current,
                Signal::derive(|| None::<String>),
                Signal::derive(|| false),
                renaming_session_id,
                Signal::derive(|| None::<String>),
                rename_draft,
            );
            let callbacks = session_sidebar_item_callbacks(
                item.id.clone(),
                item.title.clone(),
                rename_draft,
                renaming_session_id,
                item_signals.is_saving_rename,
                Callback::new(|_: (String, String)| {}),
                Callback::new(|_: String| {}),
            );

            let _ = session_sidebar_item_view(item, rename_draft, item_signals, callbacks).into_any();
        });
    }
}
