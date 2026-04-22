#![cfg_attr(not(any(test, target_family = "wasm")), allow(dead_code))]

use acp_contracts_stream::StreamEvent;
use futures_channel::mpsc;
#[cfg(target_family = "wasm")]
use wasm_bindgen::{JsCast, closure::Closure};
use web_sys::EventSource;
#[cfg(target_family = "wasm")]
use web_sys::{Event, MessageEvent};

use super::paths::session_path;

#[cfg_attr(not(target_family = "wasm"), allow(dead_code))]
pub(crate) enum SseItem {
    Event(StreamEvent),
    ParseError(String),
    Disconnected,
}

const STREAM_EVENT_NAMES: [&str; 5] = [
    "session.snapshot",
    "conversation.message",
    "tool.permission.requested",
    "session.closed",
    "status",
];

#[cfg(target_family = "wasm")]
pub(crate) fn open_session_event_stream(
    session_id: &str,
) -> Result<(EventSource, mpsc::UnboundedReceiver<SseItem>), String> {
    let event_source = EventSource::new(&session_stream_url(session_id))
        .map_err(|source| format!("Failed to open event stream: {source:?}"))?;

    let (tx, rx) = mpsc::unbounded::<SseItem>();
    register_stream_listeners(&event_source, &tx)?;
    drop(tx);

    Ok((event_source, rx))
}

#[cfg(not(target_family = "wasm"))]
pub(crate) fn open_session_event_stream(
    session_id: &str,
) -> Result<(EventSource, mpsc::UnboundedReceiver<SseItem>), String> {
    Err(non_wasm_stream_error(&session_stream_url(session_id)))
}

#[cfg(target_family = "wasm")]
fn register_stream_listeners(
    event_source: &EventSource,
    tx: &mpsc::UnboundedSender<SseItem>,
) -> Result<(), String> {
    for event_name in STREAM_EVENT_NAMES {
        register_stream_listener(event_source, event_name, tx.clone())?;
    }
    register_stream_error_listener(event_source, tx.clone());
    Ok(())
}

#[cfg(target_family = "wasm")]
fn register_stream_listener(
    event_source: &EventSource,
    event_name: &str,
    event_tx: mpsc::UnboundedSender<SseItem>,
) -> Result<(), String> {
    let listener = Closure::wrap(Box::new(move |event: MessageEvent| {
        if let Some(data) = event.data().as_string() {
            let item = match serde_json::from_str::<StreamEvent>(&data) {
                Ok(event) => SseItem::Event(event),
                Err(source) => {
                    SseItem::ParseError(format!("Failed to decode stream event: {source}"))
                }
            };
            let _ = event_tx.unbounded_send(item);
        }
    }) as Box<dyn FnMut(MessageEvent)>);

    event_source
        .add_event_listener_with_callback(event_name, listener.as_ref().unchecked_ref())
        .map_err(|source| {
            event_source.close();
            format!("Failed to register stream listener for {event_name}: {source:?}")
        })?;

    listener.forget();
    Ok(())
}

#[cfg(target_family = "wasm")]
fn register_stream_error_listener(
    event_source: &EventSource,
    error_tx: mpsc::UnboundedSender<SseItem>,
) {
    let error_listener = Closure::wrap(Box::new(move |_: Event| {
        let _ = error_tx.unbounded_send(SseItem::Disconnected);
    }) as Box<dyn FnMut(Event)>);
    event_source.set_onerror(Some(error_listener.as_ref().unchecked_ref()));
    error_listener.forget();
}

fn session_stream_url(session_id: &str) -> String {
    format!("{}/events", session_path(session_id))
}

fn non_wasm_stream_error(url: &str) -> String {
    format!("Browser event streams are unavailable on non-wasm targets: {url}")
}

#[cfg(test)]
fn stream_event_names() -> &'static [&'static str] {
    &STREAM_EVENT_NAMES
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_stream_url_uses_encoded_session_path() {
        assert_eq!(session_stream_url("s/1"), "/api/v1/sessions/s%2F1/events");
    }

    #[test]
    fn stream_event_names_cover_every_supported_event() {
        assert_eq!(
            stream_event_names(),
            &[
                "session.snapshot",
                "conversation.message",
                "tool.permission.requested",
                "session.closed",
                "status",
            ]
        );
    }

    #[test]
    fn host_event_stream_fallback_returns_descriptive_error() {
        let error = open_session_event_stream("s/1").expect_err("host stream should fail");
        assert!(error.contains("/api/v1/sessions/s%2F1/events"));
    }
}
