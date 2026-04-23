use leptos::prelude::*;

use crate::components::pending_permissions::ChatActivity;
use crate::components::transcript::{Transcript, TranscriptEntry};
use crate::session_lifecycle::SessionLifecycle;
use crate::session_page_entries::SessionEntry;

#[component]
pub(crate) fn SessionTranscriptPanel(
    #[prop(into)] entries: Signal<Vec<SessionEntry>>,
    #[prop(into)] session_status: Signal<SessionLifecycle>,
    #[prop(into)] pending_permissions: Signal<Vec<acp_contracts_permissions::PermissionRequest>>,
    #[prop(into)] pending_action_busy: Signal<bool>,
    on_approve: Callback<String>,
    on_deny: Callback<String>,
    on_cancel: Callback<()>,
) -> impl IntoView {
    let transcript_entries = session_transcript_entries(entries);

    view! {
        <div class="chat-body">
            <Transcript entries=transcript_entries />
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

fn session_transcript_entries(entries: Signal<Vec<SessionEntry>>) -> Signal<Vec<TranscriptEntry>> {
    Signal::derive(move || {
        entries
            .get()
            .into_iter()
            .map(TranscriptEntry::from)
            .collect()
    })
}

#[component]
pub(crate) fn SessionClosedNotice(
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

#[cfg(test)]
mod tests {
    use super::{SessionClosedNotice, SessionTranscriptPanel, session_transcript_entries};
    use crate::session_lifecycle::SessionLifecycle;
    use leptos::prelude::*;

    #[test]
    fn transcript_and_closed_notice_build_without_panicking() {
        let owner = Owner::new();
        owner.with(|| {
            let pending_permissions_signal = RwSignal::new(vec![acp_contracts_permissions::PermissionRequest {
                request_id: "perm-1".to_string(),
                summary: "Read file".to_string(),
            }]);
            #[rustfmt::skip]
            let pending_permissions: Signal<Vec<acp_contracts_permissions::PermissionRequest>> = Signal::derive(move || pending_permissions_signal.get());
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
        });
    }

    #[test]
    fn session_transcript_entries_maps_session_entries_for_rendering() {
        let owner = Owner::new();
        owner.with(|| {
            let entries = RwSignal::new(vec![crate::session_page_entries::SessionEntry::status(
                "status-1", "done",
            )]);
            let transcript_entries =
                session_transcript_entries(Signal::derive(move || entries.get()));

            assert_eq!(transcript_entries.get().len(), 1);
            assert_eq!(transcript_entries.get()[0].id, "status-1");
        });
    }
}
