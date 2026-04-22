use acp_contracts::StreamEvent;
use futures_channel::mpsc;
use wasm_bindgen::{JsCast, closure::Closure};
use web_sys::{Event, EventSource, MessageEvent};

use super::paths::session_path;

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

pub(crate) fn open_session_event_stream(
    session_id: &str,
) -> Result<(EventSource, mpsc::UnboundedReceiver<SseItem>), String> {
    let url = format!("{}/events", session_path(session_id));
    let event_source = EventSource::new(&url)
        .map_err(|source| format!("Failed to open event stream: {source:?}"))?;

    let (tx, rx) = mpsc::unbounded::<SseItem>();
    register_stream_listeners(&event_source, &tx)?;
    drop(tx);

    Ok((event_source, rx))
}

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

fn register_stream_listener(
    event_source: &EventSource,
    event_name: &str,
    event_tx: mpsc::UnboundedSender<SseItem>,
) -> Result<(), String> {
    let listener = Closure::wrap(Box::new(move |event: MessageEvent| {
        if let Some(data) = event.data().as_string() {
            let item = match serde_json::from_str::<StreamEvent>(&data) {
                Ok(event) => SseItem::Event(event),
                Err(source) => SseItem::ParseError(format!("Failed to decode stream event: {source}")),
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
