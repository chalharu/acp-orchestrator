#![cfg_attr(not(target_family = "wasm"), allow(dead_code))]

use futures_util::future::AbortHandle;
#[cfg(target_family = "wasm")]
use futures_util::{StreamExt, future::Abortable};
use leptos::prelude::*;

#[cfg(target_family = "wasm")]
use super::super::shared::spawn_browser_task;
#[cfg(target_family = "wasm")]
use super::events::handle_sse_event;
#[cfg(target_family = "wasm")]
use crate::infrastructure::api;
#[cfg(target_family = "wasm")]
use crate::session_lifecycle::SessionLifecycle;
use crate::session_page_signals::SessionSignals;

#[cfg(target_family = "wasm")]
pub(in crate::session_page_actions) fn spawn_session_stream(
    session_id: String,
    signals: SessionSignals,
) {
    stop_live_stream(signals);
    let (abort_handle, abort_registration) = AbortHandle::new_pair();
    signals.stream_abort.set(Some(abort_handle));
    spawn_browser_task(async move {
        let _ = Abortable::new(subscribe_sse(&session_id, signals), abort_registration).await;
        close_live_stream(signals);
        signals.stream_abort.set(None);
    });
}

#[cfg(not(target_family = "wasm"))]
pub(in crate::session_page_actions) fn spawn_session_stream(
    _session_id: String,
    signals: SessionSignals,
) {
    stop_live_stream(signals);
    let (abort_handle, _abort_registration) = AbortHandle::new_pair();
    signals.stream_abort.set(Some(abort_handle));
}

#[cfg(target_family = "wasm")]
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

    close_live_stream(signals);
}

pub(crate) fn stop_live_stream(signals: SessionSignals) {
    if let Some(abort_handle) = signals.stream_abort.get_untracked() {
        abort_handle.abort();
        signals.stream_abort.set(None);
    }
    close_live_stream(signals);
}

#[cfg(target_family = "wasm")]
fn close_live_stream(signals: SessionSignals) {
    if let Some(event_source) = signals.event_source.get_untracked() {
        event_source.close();
        signals.event_source.set(None);
    }
}

#[cfg(not(target_family = "wasm"))]
fn close_live_stream(signals: SessionSignals) {
    signals.event_source.set(None);
}

#[cfg(test)]
mod tests {
    use futures_util::future::AbortHandle;
    use leptos::prelude::*;

    use super::{spawn_session_stream, stop_live_stream};
    use crate::session_page_signals::session_signals;

    #[test]
    fn host_spawn_session_stream_sets_an_abort_handle() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            let (existing_abort, _) = AbortHandle::new_pair();
            signals.stream_abort.set(Some(existing_abort));

            spawn_session_stream("session-1".to_string(), signals);

            assert!(signals.stream_abort.get().is_some());
        });
    }

    #[test]
    fn stop_live_stream_clears_abort_state_on_host() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            let (abort_handle, _) = AbortHandle::new_pair();
            signals.stream_abort.set(Some(abort_handle));

            stop_live_stream(signals);

            assert!(signals.stream_abort.get().is_none());
            assert!(signals.event_source.get().is_none());
        });
    }
}
