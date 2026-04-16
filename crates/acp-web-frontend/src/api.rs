//! Thin async wrappers over the ACP backend REST/SSE API.
//!
//! The frontend reuses `acp-contracts` directly so the browser and backend stay
//! on the same wire schema for session snapshots, prompt submission, and named
//! SSE events.

use acp_contracts::{
    ConversationMessage, CreateSessionResponse, MessageRole, PermissionRequest, PromptRequest,
    SessionSnapshot, SessionStatus, StreamEvent, StreamEventPayload,
};
use futures_channel::mpsc;
use futures_util::StreamExt;
use gloo_net::http::Request;
use leptos::prelude::{Set, Update};
use wasm_bindgen::{JsCast, closure::Closure};
use web_sys::MessageEvent;

use crate::{EntryRole, TranscriptEntry};

pub struct SessionBootstrap {
    pub entries: Vec<TranscriptEntry>,
    pub pending_permissions: Vec<(String, String)>,
    pub session_status: String,
}

enum StreamMessage {
    Data(String),
    Error,
}

const STREAM_EVENT_NAMES: [&str; 5] = [
    "session.snapshot",
    "conversation.message",
    "tool.permission.requested",
    "session.closed",
    "status",
];

/// Read the CSRF token injected by the backend into
/// `<meta name="acp-csrf-token" content="...">`.
pub fn csrf_token() -> String {
    web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| {
            document
                .query_selector("meta[name='acp-csrf-token']")
                .ok()
                .flatten()
        })
        .and_then(|element| element.get_attribute("content"))
        .unwrap_or_default()
}

/// Create a new session and immediately post the first prompt.
pub async fn create_session_and_send(prompt: &str) -> Result<String, String> {
    let csrf = csrf_token();
    let response = Request::post("/api/v1/sessions")
        .header("x-csrf-token", &csrf)
        .send()
        .await
        .map_err(|error| error.to_string())?;

    if !response.ok() {
        return Err(format!("Create session failed: HTTP {}", response.status()));
    }

    let created: CreateSessionResponse =
        response.json().await.map_err(|error| error.to_string())?;
    let session_id = created.session.id.clone();
    send_message(&session_id, prompt).await?;
    Ok(session_id)
}

/// Load the current snapshot for an existing session.
pub async fn load_session(session_id: &str) -> Result<SessionBootstrap, String> {
    let url = format!("/api/v1/sessions/{session_id}");
    let response = Request::get(&url)
        .send()
        .await
        .map_err(|error| error.to_string())?;

    if !response.ok() {
        return Err(format!("Load session failed: HTTP {}", response.status()));
    }

    let session: SessionSnapshot = response.json().await.map_err(|error| error.to_string())?;
    let SessionSnapshot {
        status,
        messages,
        pending_permissions,
        ..
    } = session;

    Ok(SessionBootstrap {
        entries: messages.into_iter().map(message_to_entry).collect(),
        pending_permissions: pending_permissions_to_items(pending_permissions),
        session_status: session_status_label(status).to_string(),
    })
}

/// POST a new message to an existing session.
pub async fn send_message(session_id: &str, text: &str) -> Result<(), String> {
    let csrf = csrf_token();
    let url = format!("/api/v1/sessions/{session_id}/messages");
    let body = serde_json::to_string(&PromptRequest {
        text: text.to_string(),
    })
    .map_err(|error| error.to_string())?;

    let response = Request::post(&url)
        .header("x-csrf-token", &csrf)
        .header("content-type", "application/json")
        .body(body)
        .map_err(|error| error.to_string())?
        .send()
        .await
        .map_err(|error| error.to_string())?;

    if !response.ok() {
        return Err(format!("Send message failed: HTTP {}", response.status()));
    }
    Ok(())
}

/// Open the session event stream and keep driving the supplied signals until
/// the page unloads or the stream fatally fails.
pub async fn subscribe_sse(
    session_id: &str,
    entries: leptos::prelude::RwSignal<Vec<TranscriptEntry>>,
    pending_permissions_signal: leptos::prelude::RwSignal<Vec<(String, String)>>,
    connection_status: leptos::prelude::RwSignal<String>,
    session_status: leptos::prelude::RwSignal<String>,
    error: leptos::prelude::RwSignal<Option<String>>,
) {
    let url = format!("/api/v1/sessions/{session_id}/events");
    let Some(event_source) = open_event_source(&url, error, connection_status) else {
        return;
    };

    let (tx, mut rx) = mpsc::unbounded::<StreamMessage>();
    if !register_stream_listeners(&event_source, &tx, error, connection_status) {
        return;
    }
    drop(tx);
    connection_status.set("connected".to_string());

    while let Some(message) = rx.next().await {
        if !handle_stream_message(
            message,
            &event_source,
            entries,
            pending_permissions_signal,
            connection_status,
            session_status,
            error,
        ) {
            return;
        }
    }

    event_source.close();
}

fn open_event_source(
    url: &str,
    error: leptos::prelude::RwSignal<Option<String>>,
    connection_status: leptos::prelude::RwSignal<String>,
) -> Option<web_sys::EventSource> {
    match web_sys::EventSource::new(url) {
        Ok(event_source) => Some(event_source),
        Err(source) => {
            error.set(Some(format!("Failed to open event stream: {source:?}")));
            connection_status.set("error".to_string());
            None
        }
    }
}

fn register_stream_listeners(
    event_source: &web_sys::EventSource,
    tx: &mpsc::UnboundedSender<StreamMessage>,
    error: leptos::prelude::RwSignal<Option<String>>,
    connection_status: leptos::prelude::RwSignal<String>,
) -> bool {
    for event_name in STREAM_EVENT_NAMES {
        if !register_stream_listener(
            event_source,
            event_name,
            tx.clone(),
            error,
            connection_status,
        ) {
            return false;
        }
    }
    register_stream_error_listener(event_source, tx.clone());
    true
}

fn register_stream_listener(
    event_source: &web_sys::EventSource,
    event_name: &str,
    event_tx: mpsc::UnboundedSender<StreamMessage>,
    error: leptos::prelude::RwSignal<Option<String>>,
    connection_status: leptos::prelude::RwSignal<String>,
) -> bool {
    let listener = Closure::wrap(Box::new(move |event: MessageEvent| {
        if let Some(data) = event.data().as_string() {
            let _ = event_tx.unbounded_send(StreamMessage::Data(data));
        }
    }) as Box<dyn FnMut(MessageEvent)>);

    if let Err(source) =
        event_source.add_event_listener_with_callback(event_name, listener.as_ref().unchecked_ref())
    {
        event_source.close();
        error.set(Some(format!(
            "Failed to register stream listener for {event_name}: {source:?}"
        )));
        connection_status.set("error".to_string());
        return false;
    }

    listener.forget();
    true
}

fn register_stream_error_listener(
    event_source: &web_sys::EventSource,
    error_tx: mpsc::UnboundedSender<StreamMessage>,
) {
    let error_listener = Closure::wrap(Box::new(move |_: web_sys::Event| {
        let _ = error_tx.unbounded_send(StreamMessage::Error);
    }) as Box<dyn FnMut(web_sys::Event)>);
    event_source.set_onerror(Some(error_listener.as_ref().unchecked_ref()));
    error_listener.forget();
}

fn handle_stream_message(
    message: StreamMessage,
    event_source: &web_sys::EventSource,
    entries: leptos::prelude::RwSignal<Vec<TranscriptEntry>>,
    pending_permissions_signal: leptos::prelude::RwSignal<Vec<(String, String)>>,
    connection_status: leptos::prelude::RwSignal<String>,
    session_status: leptos::prelude::RwSignal<String>,
    error: leptos::prelude::RwSignal<Option<String>>,
) -> bool {
    match message {
        StreamMessage::Data(data) => handle_stream_data(
            &data,
            event_source,
            entries,
            pending_permissions_signal,
            connection_status,
            session_status,
            error,
        ),
        StreamMessage::Error => {
            connection_status.set("reconnecting".to_string());
            error.set(Some("Event stream disconnected; reconnecting...".to_string()));
            true
        }
    }
}

fn handle_stream_data(
    data: &str,
    event_source: &web_sys::EventSource,
    entries: leptos::prelude::RwSignal<Vec<TranscriptEntry>>,
    pending_permissions_signal: leptos::prelude::RwSignal<Vec<(String, String)>>,
    connection_status: leptos::prelude::RwSignal<String>,
    session_status: leptos::prelude::RwSignal<String>,
    error: leptos::prelude::RwSignal<Option<String>>,
) -> bool {
    let event = match serde_json::from_str::<StreamEvent>(data) {
        Ok(event) => event,
        Err(source) => {
            event_source.close();
            error.set(Some(format!("Failed to decode stream event: {source}")));
            connection_status.set("error".to_string());
            return false;
        }
    };

    connection_status.set("connected".to_string());
    error.set(None);
    handle_sse_event(event, entries, pending_permissions_signal, session_status);
    true
}

fn handle_sse_event(
    event: StreamEvent,
    entries: leptos::prelude::RwSignal<Vec<TranscriptEntry>>,
    pending_permissions_signal: leptos::prelude::RwSignal<Vec<(String, String)>>,
    session_status: leptos::prelude::RwSignal<String>,
) {
    let StreamEvent { sequence, payload } = event;

    match payload {
        StreamEventPayload::SessionSnapshot { session } => {
            let SessionSnapshot {
                status,
                messages,
                pending_permissions,
                ..
            } = session;
            session_status.set(session_status_label(status).to_string());
            pending_permissions_signal.set(pending_permissions_to_items(pending_permissions));
            entries.set(messages.into_iter().map(message_to_entry).collect());
        }
        StreamEventPayload::ConversationMessage { message } => {
            entries.update(|current_entries| {
                if !current_entries.iter().any(|entry| entry.id == message.id) {
                    current_entries.push(message_to_entry(message));
                }
            });
        }
        StreamEventPayload::PermissionRequested { request } => {
            pending_permissions_signal.update(|current_permissions| {
                if !current_permissions
                    .iter()
                    .any(|(request_id, _)| request_id == &request.request_id)
                {
                    current_permissions.push((request.request_id, request.summary));
                }
            });
        }
        StreamEventPayload::SessionClosed { reason, .. } => {
            session_status.set("closed".to_string());
            push_status_entry(entries, sequence, reason);
        }
        StreamEventPayload::Status { message } => {
            push_status_entry(entries, sequence, message);
        }
    }
}

fn pending_permissions_to_items(
    pending_permissions: Vec<PermissionRequest>,
) -> Vec<(String, String)> {
    pending_permissions
        .into_iter()
        .map(|request| (request.request_id, request.summary))
        .collect()
}

fn push_status_entry(
    entries: leptos::prelude::RwSignal<Vec<TranscriptEntry>>,
    sequence: u64,
    text: String,
) {
    if text.trim().is_empty() {
        return;
    }

    let entry_id = format!("status-{sequence}");
    entries.update(|current_entries| {
        if current_entries.iter().any(|entry| entry.id == entry_id) {
            return;
        }

        current_entries.push(TranscriptEntry {
            id: entry_id.clone(),
            role: EntryRole::Status,
            text: text.clone(),
        });
    });
}

fn message_to_entry(message: ConversationMessage) -> TranscriptEntry {
    TranscriptEntry {
        id: message.id,
        role: message_role(message.role),
        text: message.text,
    }
}

fn message_role(role: MessageRole) -> EntryRole {
    match role {
        MessageRole::User => EntryRole::User,
        MessageRole::Assistant => EntryRole::Assistant,
    }
}

fn session_status_label(status: SessionStatus) -> &'static str {
    match status {
        SessionStatus::Active => "active",
        SessionStatus::Closed => "closed",
    }
}
