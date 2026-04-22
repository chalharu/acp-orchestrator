use leptos::prelude::*;
#[cfg(target_family = "wasm")]
use leptos::portal::Portal;

use crate::components::error_banner::ErrorBanner;

use super::main::SessionMain;
use super::sidebar::SessionSidebar;
use super::super::{
    actions::{
        bind_slash_completion, delete_session_callback, rename_session_callback,
        session_permission_callbacks, session_submit_callback, slash_palette_callbacks,
        spawn_home_redirect, spawn_session_bootstrap,
    },
    state::{
        SessionComposerSignals, SessionMainSignals, SessionShellSignals, SessionSignals,
        SessionViewCallbacks,
        current_session_deleting_signal, persist_session_draft, restore_session_draft,
        session_composer_signals, session_main_signals, session_shell_signals, session_signals,
    },
};

/// Landing page. Prepares a fresh session and immediately redirects to the
/// live chat route so startup hints appear before the first prompt.
#[cfg(target_family = "wasm")]
pub(super) fn bind_home_redirect(
    started: RwSignal<bool>,
    error: RwSignal<Option<String>>,
    preparing: RwSignal<bool>,
) {
    Effect::new(move |_| {
        if started.get() {
            return;
        }

        started.set(true);
        error.set(None);
        spawn_home_redirect(error, preparing);
    });
}

#[cfg(not(target_family = "wasm"))]
pub(super) fn bind_home_redirect(
    started: RwSignal<bool>,
    error: RwSignal<Option<String>>,
    preparing: RwSignal<bool>,
) {
    if started.get_untracked() {
        return;
    }

    started.set(true);
    error.set(None);
    spawn_home_redirect(error, preparing);
}

pub(super) fn home_message(preparing: bool) -> &'static str {
    if preparing {
        "Preparing chat..."
    } else {
        "Unable to prepare a new chat."
    }
}

#[cfg(target_family = "wasm")]
#[component]
pub(crate) fn HomePage() -> impl IntoView {
    let error = RwSignal::new(None::<String>);
    let preparing = RwSignal::new(true);
    let started = RwSignal::new(false);

    bind_home_redirect(started, error, preparing);

    view! {
        <main class="app-shell app-shell--home">
            <ErrorBanner message=error />
            <section class="panel empty-state">
                <p class="muted">{move || home_message(preparing.get())}</p>
            </section>
        </main>
    }
}

#[cfg(not(target_family = "wasm"))]
#[component]
pub(crate) fn HomePage() -> impl IntoView {
    let error = RwSignal::new(None::<String>);
    let preparing = RwSignal::new(true);
    let started = RwSignal::new(false);

    bind_home_redirect(started, error, preparing);

    let home_message = home_message(preparing.get_untracked());

    view! {
        <main class="app-shell app-shell--home">
            <ErrorBanner message=error />
            <section class="panel empty-state">
                <p class="muted">{home_message}</p>
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

pub(super) fn session_view_callbacks(
    session_id: String,
    signals: SessionSignals,
) -> SessionViewCallbacks {
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
pub(super) fn SessionShell(
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

#[cfg(target_family = "wasm")]
#[component]
pub(super) fn SessionBackdrop(sidebar_open: RwSignal<bool>) -> impl IntoView {
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

#[cfg(not(target_family = "wasm"))]
#[component]
pub(super) fn SessionBackdrop(sidebar_open: RwSignal<bool>) -> impl IntoView {
    let is_hidden = !sidebar_open.get_untracked();

    view! {
        <div class="session-layout__backdrop" hidden=is_hidden></div>
    }
}

#[cfg(target_family = "wasm")]
pub(super) fn default_sidebar_open() -> bool {
    web_sys::window()
        .and_then(|window| window.inner_width().ok())
        .and_then(|width| width.as_f64())
        .map(|width| width >= 960.0)
        .unwrap_or(true)
}

#[cfg(not(target_family = "wasm"))]
pub(super) fn default_sidebar_open() -> bool {
    true
}

pub(super) fn session_layout_class(sidebar_open: bool) -> &'static str {
    if sidebar_open {
        "session-layout session-layout--sidebar-open"
    } else {
        "session-layout"
    }
}

#[cfg(test)]
mod tests {
    use leptos::prelude::*;

    use super::{
        HomePage, SessionBackdrop, SessionView, bind_home_redirect, default_sidebar_open,
        home_message, session_layout_class, session_view_callbacks,
    };
    use crate::session::page::state::session_signals;

    #[test]
    fn session_layout_class_adds_open_modifier_when_sidebar_is_open() {
        assert_eq!(
            session_layout_class(true),
            "session-layout session-layout--sidebar-open"
        );
        assert_eq!(session_layout_class(false), "session-layout");
    }

    #[test]
    fn default_sidebar_open_returns_true_without_browser_window() {
        assert!(default_sidebar_open());
    }

    #[test]
    fn home_page_builds_without_panicking_on_host() {
        let owner = Owner::new();
        owner.with(|| {
            let _ = view! { <HomePage /> };
        });
    }

    #[test]
    fn session_view_builds_without_panicking_on_host() {
        let owner = Owner::new();
        owner.with(|| {
            let _ = view! { <SessionView session_id="session-1".to_string() /> };
        });
    }

    #[test]
    fn session_view_callbacks_build_without_panicking() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            let _ = session_view_callbacks("session-1".to_string(), signals);
        });
    }

    #[test]
    fn session_backdrop_builds_without_panicking() {
        let owner = Owner::new();
        owner.with(|| {
            let sidebar_open = RwSignal::new(false);
            let _ = view! { <SessionBackdrop sidebar_open=sidebar_open /> };
        });
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn bind_home_redirect_ignores_second_call_after_start() {
        let owner = Owner::new();
        owner.with(|| {
            let started = RwSignal::new(false);
            let error = RwSignal::new(Some("old error".to_string()));
            let preparing = RwSignal::new(true);

            bind_home_redirect(started, error, preparing);
            assert!(started.get());
            assert!(error.get().is_none());

            error.set(Some("keep me".to_string()));
            bind_home_redirect(started, error, preparing);
            assert_eq!(error.get(), Some("keep me".to_string()));
        });
    }

    #[test]
    fn home_message_covers_both_preparing_states() {
        assert_eq!(home_message(true), "Preparing chat...");
        assert_eq!(home_message(false), "Unable to prepare a new chat.");
    }
}
