use leptos::prelude::*;
#[cfg(target_family = "wasm")]
use wasm_bindgen::JsCast;

use crate::session_page_sidebar_styles::{
    session_sidebar_item_class, session_sidebar_status_label, session_sidebar_status_pill_class,
};
#[cfg(not(target_family = "wasm"))]
use crate::session_page_sidebar_styles::sidebar_delete_sr_label;

#[derive(Clone, Debug, PartialEq, Eq)]
struct SessionSidebarItemModel {
    id: String,
    href: String,
    title: String,
    activity_label: String,
    is_current: bool,
    is_closed: bool,
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

#[component]
pub(super) fn SessionSidebarItem(
    id: String,
    href: String,
    title: String,
    activity_label: String,
    is_current: bool,
    is_closed: bool,
    #[prop(into)] deleting_session_id: Signal<Option<String>>,
    #[prop(into)] delete_disabled: Signal<bool>,
    renaming_session_id: RwSignal<Option<String>>,
    #[prop(into)] saving_rename_session_id: Signal<Option<String>>,
    rename_draft: RwSignal<String>,
    on_rename_session: Callback<(String, String)>,
    on_delete_session: Callback<String>,
) -> impl IntoView {
    let item = SessionSidebarItemModel {
        id,
        href,
        title,
        activity_label,
        is_current,
        is_closed,
    };
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

#[cfg(target_family = "wasm")]
fn session_sidebar_item_view(
    item: SessionSidebarItemModel,
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
fn session_sidebar_item_view(
    item: SessionSidebarItemModel,
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

#[cfg(test)]
mod tests {
    use leptos::prelude::*;

    use super::*;

    fn sample_sidebar_session() -> SessionSidebarItemModel {
        SessionSidebarItemModel {
            id: "s1".to_string(),
            href: "/app/sessions/s1".to_string(),
            title: "Test session".to_string(),
            activity_label: "Updated now".to_string(),
            is_current: true,
            is_closed: false,
        }
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
    fn sidebar_rename_form_builds_without_panicking() {
        let owner = Owner::new();
        owner.with(|| {
            let rename_draft = RwSignal::new("Draft".to_string());

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

            let _ =
                session_sidebar_item_view(item, rename_draft, item_signals, callbacks).into_any();
        });
    }
}
