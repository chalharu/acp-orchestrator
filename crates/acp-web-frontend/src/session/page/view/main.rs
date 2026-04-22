use acp_contracts_permissions::PermissionRequest;
use leptos::prelude::*;

use crate::components::composer::Composer;
use crate::components::error_banner::ErrorBanner;
use crate::components::pending_permissions::ChatActivity;
use crate::components::transcript::Transcript;
use crate::session_lifecycle::SessionLifecycle;

use super::super::entries::SessionEntry;
use super::super::state::{
    SessionComposerSignals, SessionMainSignals, SessionViewCallbacks, StatusBadge,
};

#[component]
pub(super) fn SessionMain(
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
pub(super) fn SessionTranscriptPanel(
    #[prop(into)] entries: Signal<Vec<SessionEntry>>,
    #[prop(into)] session_status: Signal<SessionLifecycle>,
    #[prop(into)] pending_permissions: Signal<Vec<PermissionRequest>>,
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
pub(super) fn SessionClosedNotice(
    #[prop(into)] session_status: Signal<SessionLifecycle>,
) -> impl IntoView {
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

#[cfg(target_family = "wasm")]
#[component]
pub(super) fn SessionTopBar(
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

#[cfg(not(target_family = "wasm"))]
#[component]
pub(super) fn SessionTopBar(
    #[prop(into)] message: Signal<Option<String>>,
    #[prop(into)] connection_badge: Signal<StatusBadge>,
    #[prop(into)] worker_badge: Signal<StatusBadge>,
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
pub(super) fn StatusBadgeView(#[prop(into)] badge: Signal<StatusBadge>) -> impl IntoView {
    view! {
        <p class=move || status_badge_class(badge.get())>
            <span class="status-badge__label">{move || badge.get().label}</span>
            <span class="status-badge__value">{move || badge.get().value}</span>
        </p>
    }
}

#[component]
pub(super) fn SessionDock(
    composer: SessionComposerSignals,
    callbacks: SessionViewCallbacks,
    draft: RwSignal<String>,
) -> impl IntoView {
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
                slash_visible=composer.slash_palette_visible
                slash_candidates=composer.slash_candidates
                slash_selected_index=composer.slash_selected_index
                slash_apply_selected=composer.slash_apply_selected
                on_slash_select_next=callbacks.slash.select_next
                on_slash_select_previous=callbacks.slash.select_previous
                on_slash_apply_selected=callbacks.slash.apply_selected
                on_slash_apply_index=callbacks.slash.apply_index
                on_slash_dismiss=callbacks.slash.dismiss
            />
        </div>
    }
}

#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn topbar_toggle_icon(sidebar_open: bool) -> &'static str {
    if sidebar_open { "←" } else { "☰" }
}

#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn topbar_toggle_label(sidebar_open: bool) -> &'static str {
    if sidebar_open {
        "Hide sessions"
    } else {
        "Show sessions"
    }
}

fn status_badge_class(badge: StatusBadge) -> &'static str {
    match badge.tone {
        crate::session_lifecycle::BadgeTone::Neutral => "status-badge status-badge--neutral",
        crate::session_lifecycle::BadgeTone::Success => "status-badge status-badge--success",
        crate::session_lifecycle::BadgeTone::Warning => "status-badge status-badge--warning",
        crate::session_lifecycle::BadgeTone::Danger => "status-badge status-badge--danger",
    }
}

#[cfg(test)]
mod tests {
    use leptos::prelude::*;

    use super::{
        SessionClosedNotice, SessionDock, SessionMain, SessionTopBar, SessionTranscriptPanel,
        StatusBadgeView, topbar_toggle_icon, topbar_toggle_label,
    };
    use crate::session_lifecycle::{BadgeTone, SessionLifecycle};
    use crate::session::page::state::{
        StatusBadge, session_composer_signals, session_main_signals, session_signals,
    };
    use crate::session::page::view::layout::session_view_callbacks;
    use acp_contracts_permissions::PermissionRequest;

    fn badge(label: &'static str, value: &'static str, tone: BadgeTone) -> StatusBadge {
        StatusBadge { label, value, tone }
    }

    #[test]
    fn topbar_labels_match_sidebar_state() {
        assert_eq!(topbar_toggle_icon(true), "←");
        assert_eq!(topbar_toggle_icon(false), "☰");
        assert_eq!(topbar_toggle_label(true), "Hide sessions");
        assert_eq!(topbar_toggle_label(false), "Show sessions");
    }

    #[test]
    fn transcript_and_badge_components_build_without_panicking() {
        let owner = Owner::new();
        owner.with(|| {
            let pending_permissions_signal = RwSignal::new(vec![PermissionRequest {
                request_id: "perm-1".to_string(),
                summary: "Read file".to_string(),
            }]);
            #[rustfmt::skip]
            let pending_permissions: Signal<Vec<PermissionRequest>> = Signal::derive(move || pending_permissions_signal.get());
            let _ = view! {
                <SessionTranscriptPanel
                    entries=Signal::derive(Vec::new)
                    session_status=Signal::derive(|| SessionLifecycle::Closed)
                    pending_permissions
                    pending_action_busy=Signal::derive(|| false)
                    on_approve=Callback::new(|_: String| {})
                    on_deny=Callback::new(|_: String| {})
                    on_cancel=Callback::new(|()| {})
                />
            };
            let _ = view! { <SessionClosedNotice session_status=Signal::derive(|| SessionLifecycle::Closed) /> };
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

    #[test]
    fn session_dock_builds_without_panicking() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            let draft = signals.draft;
            let composer = session_composer_signals(signals, Signal::derive(|| false));
            let callbacks = session_view_callbacks("s1".to_string(), signals);

            let _ = view! {
                <SessionDock
                    composer=composer
                    callbacks=callbacks
                    draft=draft
                />
            };
        });
    }

    #[test]
    fn session_main_builds_without_panicking() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            let draft = signals.draft;
            let composer = session_composer_signals(signals, Signal::derive(|| false));
            let main_signals = session_main_signals(signals);
            let callbacks = session_view_callbacks("s1".to_string(), signals);
            let sidebar_open = RwSignal::new(false);

            let _ = view! {
                <SessionMain
                    main_signals=main_signals
                    sidebar_open=sidebar_open
                    composer=composer
                    callbacks=callbacks
                    draft=draft
                />
            };
        });
    }

    #[test]
    fn badge_helper_builds_status_badge() {
        let badge = badge("Connection", "live", BadgeTone::Success);
        assert_eq!(badge.label, "Connection");
        assert_eq!(badge.value, "live");
        assert_eq!(badge.tone, BadgeTone::Success);
    }
}
