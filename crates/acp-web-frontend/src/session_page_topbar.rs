use leptos::prelude::*;

use crate::components::error_banner::ErrorBanner;
use crate::session_lifecycle::BadgeTone;

#[cfg(target_family = "wasm")]
#[component]
pub(crate) fn SessionTopBar(
    #[prop(into)] message: Signal<Option<String>>,
    #[prop(into)] connection_badge: Signal<(&'static str, &'static str, BadgeTone)>,
    #[prop(into)] worker_badge: Signal<(&'static str, &'static str, BadgeTone)>,
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

#[cfg(not(target_family = "wasm"))]
#[component]
pub(crate) fn SessionTopBar(
    #[prop(into)] message: Signal<Option<String>>,
    #[prop(into)] connection_badge: Signal<(&'static str, &'static str, BadgeTone)>,
    #[prop(into)] worker_badge: Signal<(&'static str, &'static str, BadgeTone)>,
    sidebar_open: RwSignal<bool>,
) -> impl IntoView {
    let sidebar_open_now = sidebar_open.get_untracked();

    view! {
        <div class="chat-topbar">
            <div class="chat-topbar__controls">
                <button
                    class="session-sidebar__toggle"
                    type="button"
                    aria-expanded=if sidebar_open_now { "true" } else { "false" }
                >
                    <span class="sidebar-toggle-icon" aria-hidden="true">
                        {topbar_toggle_icon(sidebar_open_now)}
                    </span>
                    <span class="session-sidebar__toggle-label">
                        {topbar_toggle_label(sidebar_open_now)}
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
pub(crate) fn StatusBadgeView(
    #[prop(into)] badge: Signal<(&'static str, &'static str, BadgeTone)>,
) -> impl IntoView {
    view! {
        <p class=move || status_badge_class(badge.get())>
            <span class="status-badge__label">{move || badge.get().0}</span>
            <span class="status-badge__value">{move || badge.get().1}</span>
        </p>
    }
}

pub(crate) fn topbar_toggle_icon(sidebar_open: bool) -> &'static str {
    if sidebar_open { "←" } else { "☰" }
}

pub(crate) fn topbar_toggle_label(sidebar_open: bool) -> &'static str {
    if sidebar_open {
        "Hide sessions"
    } else {
        "Show sessions"
    }
}

fn status_badge_class((_, _, tone): (&'static str, &'static str, BadgeTone)) -> &'static str {
    match tone {
        BadgeTone::Neutral => "status-badge status-badge--neutral",
        BadgeTone::Success => "status-badge status-badge--success",
        BadgeTone::Warning => "status-badge status-badge--warning",
        BadgeTone::Danger => "status-badge status-badge--danger",
    }
}

#[cfg(test)]
mod tests {
    use super::{
        StatusBadgeView, SessionTopBar, status_badge_class, topbar_toggle_icon, topbar_toggle_label,
    };
    use crate::session_lifecycle::BadgeTone;
    use crate::session_page_main_signals::tests::badge;
    use leptos::prelude::*;

    #[test]
    fn topbar_labels_match_sidebar_state() {
        assert_eq!(topbar_toggle_icon(true), "←");
        assert_eq!(topbar_toggle_icon(false), "☰");
        assert_eq!(topbar_toggle_label(true), "Hide sessions");
        assert_eq!(topbar_toggle_label(false), "Show sessions");
    }

    #[test]
    fn status_badge_class_matches_badge_tone() {
        assert_eq!(
            status_badge_class(badge("Connection", "live", BadgeTone::Neutral)),
            "status-badge status-badge--neutral"
        );
        assert_eq!(
            status_badge_class(badge("Connection", "live", BadgeTone::Success)),
            "status-badge status-badge--success"
        );
        assert_eq!(
            status_badge_class(badge("Connection", "live", BadgeTone::Warning)),
            "status-badge status-badge--warning"
        );
        assert_eq!(
            status_badge_class(badge("Connection", "live", BadgeTone::Danger)),
            "status-badge status-badge--danger"
        );
    }

    #[test]
    fn topbar_and_badge_components_build_without_panicking() {
        let owner = Owner::new();
        owner.with(|| {
            let _ = view! {
                <StatusBadgeView
                    badge=Signal::derive(|| badge("Connection", "live", BadgeTone::Success))
                />
            };
            let _ = view! {
                <SessionTopBar
                    message=Signal::derive(|| Some("warning".to_string()))
                    connection_badge=Signal::derive(|| badge("Connection", "live", BadgeTone::Success))
                    worker_badge=Signal::derive(|| badge("Worker", "idle", BadgeTone::Neutral))
                    sidebar_open=RwSignal::new(false)
                />
            };
        });
    }
}
