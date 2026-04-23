#[cfg(target_family = "wasm")]
use leptos::portal::Portal;
use leptos::prelude::*;

use crate::components::error_banner::ErrorBanner;
use crate::session_page_actions::spawn_home_redirect;

/// Landing page. Prepares a fresh session and immediately redirects to the
/// live chat route so startup hints appear before the first prompt.
#[cfg(target_family = "wasm")]
pub(crate) fn bind_home_redirect(
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
pub(crate) fn bind_home_redirect(
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

pub(crate) fn home_message(preparing: bool) -> &'static str {
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

#[cfg(target_family = "wasm")]
#[component]
pub(crate) fn SessionBackdrop(sidebar_open: RwSignal<bool>) -> impl IntoView {
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
pub(crate) fn SessionBackdrop(sidebar_open: RwSignal<bool>) -> impl IntoView {
    let is_hidden = !sidebar_open.get_untracked();

    view! {
        <div class="session-layout__backdrop" hidden=is_hidden></div>
    }
}

#[cfg(target_family = "wasm")]
pub(crate) fn default_sidebar_open() -> bool {
    web_sys::window()
        .and_then(|window| window.inner_width().ok())
        .and_then(|width| width.as_f64())
        .map(|width| width >= 960.0)
        .unwrap_or(true)
}

#[cfg(not(target_family = "wasm"))]
pub(crate) fn default_sidebar_open() -> bool {
    true
}

pub(crate) fn session_layout_class(sidebar_open: bool) -> &'static str {
    if sidebar_open {
        "session-layout session-layout--sidebar-open"
    } else {
        "session-layout"
    }
}

#[cfg(test)]
mod tests {
    use super::{
        HomePage, SessionBackdrop, bind_home_redirect, default_sidebar_open, home_message,
        session_layout_class,
    };
    use leptos::prelude::*;

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
