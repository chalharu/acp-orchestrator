//! Thin async wrappers over the ACP backend REST/SSE API.
//!
//! The frontend reuses `acp-contracts` directly so the browser and backend stay
//! on the same wire schema for session snapshots, prompt submission, and named
//! SSE events.

use acp_contracts::{
    CancelTurnResponse, ConversationMessage, CreateSessionResponse, ErrorResponse, MessageRole,
    PermissionDecision, PermissionRequest, PromptRequest, ResolvePermissionRequest,
    SessionResponse, SessionSnapshot, SessionStatus, StreamEvent, StreamEventPayload,
};
use futures_channel::mpsc;
use futures_util::StreamExt;
use gloo_net::http::Request;
use leptos::prelude::{GetUntracked, Set, Update};
use wasm_bindgen::{JsCast, closure::Closure};
use web_sys::MessageEvent;

use crate::{
    EntryRole, TranscriptEntry, TurnState, should_apply_snapshot_turn_state,
    should_release_turn_state_for_assistant_message, should_release_turn_state_for_status,
    turn_state_for_snapshot,
};

pub struct SessionBootstrap {
    pub entries: Vec<TranscriptEntry>,
    pub pending_permissions: Vec<(String, String)>,
    pub session_status: String,
}

#[derive(Clone, Copy)]
struct StreamSignals {
    entries: leptos::prelude::RwSignal<Vec<TranscriptEntry>>,
    pending_permissions: leptos::prelude::RwSignal<Vec<(String, String)>>,
    connection_status: leptos::prelude::RwSignal<String>,
    session_status: leptos::prelude::RwSignal<String>,
    turn_state: leptos::prelude::RwSignal<TurnState>,
    error: leptos::prelude::RwSignal<Option<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionLoadError {
    ResumeUnavailable(String),
    Other(String),
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

/// Create a new session.
pub async fn create_session() -> Result<String, String> {
    let csrf = csrf_token();
    let response = Request::post("/api/v1/sessions")
        .header("x-csrf-token", &csrf)
        .send()
        .await
        .map_err(|error| error.to_string())?;

    if !response.ok() {
        return Err(response_error_message(response, "Create session failed").await);
    }

    let created: CreateSessionResponse =
        response.json().await.map_err(|error| error.to_string())?;
    Ok(created.session.id)
}

/// Load the current snapshot for an existing session.
pub async fn load_session(session_id: &str) -> Result<SessionBootstrap, SessionLoadError> {
    let url = format!("/api/v1/sessions/{session_id}");
    let response = Request::get(&url)
        .send()
        .await
        .map_err(|error| SessionLoadError::Other(error.to_string()))?;

    if !response.ok() {
        return Err(classify_session_load_failure(response).await);
    }

    let session: SessionResponse = response
        .json()
        .await
        .map_err(|error| SessionLoadError::Other(error.to_string()))?;

    Ok(session_bootstrap_from_snapshot(session.session))
}

/// POST a new message to an existing session.
pub async fn send_message(session_id: &str, text: &str) -> Result<(), String> {
    let url = format!("/api/v1/sessions/{session_id}/messages");
    let body = serde_json::to_string(&PromptRequest {
        text: text.to_string(),
    })
    .map_err(|error| error.to_string())?;

    let response = post_json_with_csrf(&url, body).await?;

    if !response.ok() {
        return Err(response_error_message(response, "Send message failed").await);
    }
    Ok(())
}

pub async fn resolve_permission(
    session_id: &str,
    request_id: &str,
    decision: PermissionDecision,
) -> Result<(), String> {
    let url = format!("/api/v1/sessions/{session_id}/permissions/{request_id}");
    let body = serde_json::to_string(&ResolvePermissionRequest { decision })
        .map_err(|error| error.to_string())?;

    let response = post_json_with_csrf(&url, body).await?;

    if !response.ok() {
        return Err(response_error_message(response, "Resolve permission failed").await);
    }

    Ok(())
}

pub async fn cancel_turn(session_id: &str) -> Result<CancelTurnResponse, String> {
    let csrf = csrf_token();
    let url = format!("/api/v1/sessions/{session_id}/cancel");
    let response = Request::post(&url)
        .header("x-csrf-token", &csrf)
        .send()
        .await
        .map_err(|error| error.to_string())?;

    if !response.ok() {
        return Err(response_error_message(response, "Cancel turn failed").await);
    }

    response.json().await.map_err(|error| error.to_string())
}

async fn post_json_with_csrf(url: &str, body: String) -> Result<gloo_net::http::Response, String> {
    let csrf = csrf_token();
    Request::post(url)
        .header("x-csrf-token", &csrf)
        .header("content-type", "application/json")
        .body(body)
        .map_err(|error| error.to_string())?
        .send()
        .await
        .map_err(|error| error.to_string())
}

/// Open the session event stream and keep driving the supplied signals until
/// the page unloads or the stream fatally fails.
pub async fn subscribe_sse(
    session_id: &str,
    entries: leptos::prelude::RwSignal<Vec<TranscriptEntry>>,
    pending_permissions_signal: leptos::prelude::RwSignal<Vec<(String, String)>>,
    connection_status: leptos::prelude::RwSignal<String>,
    session_status: leptos::prelude::RwSignal<String>,
    turn_state: leptos::prelude::RwSignal<TurnState>,
    error: leptos::prelude::RwSignal<Option<String>>,
) {
    let signals = StreamSignals {
        entries,
        pending_permissions: pending_permissions_signal,
        connection_status,
        session_status,
        turn_state,
        error,
    };
    let url = format!("/api/v1/sessions/{session_id}/events");
    let Some(event_source) = open_event_source(&url, signals.error, signals.connection_status)
    else {
        return;
    };

    let (tx, mut rx) = mpsc::unbounded::<StreamMessage>();
    if !register_stream_listeners(&event_source, &tx, signals.error, signals.connection_status) {
        return;
    }
    drop(tx);
    signals.connection_status.set("connected".to_string());

    while let Some(message) = rx.next().await {
        if !handle_stream_message(message, &event_source, signals) {
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
    signals: StreamSignals,
) -> bool {
    match message {
        StreamMessage::Data(data) => handle_stream_data(&data, event_source, signals),
        StreamMessage::Error => {
            signals.connection_status.set("reconnecting".to_string());
            signals.error.set(Some(
                "Event stream disconnected; reconnecting...".to_string(),
            ));
            true
        }
    }
}

fn handle_stream_data(
    data: &str,
    event_source: &web_sys::EventSource,
    signals: StreamSignals,
) -> bool {
    let event = match serde_json::from_str::<StreamEvent>(data) {
        Ok(event) => event,
        Err(source) => {
            event_source.close();
            signals
                .error
                .set(Some(format!("Failed to decode stream event: {source}")));
            signals.connection_status.set("error".to_string());
            return false;
        }
    };

    signals.connection_status.set("connected".to_string());
    signals.error.set(None);
    handle_sse_event(event, signals);
    true
}

fn handle_sse_event(event: StreamEvent, signals: StreamSignals) {
    let StreamEvent { sequence, payload } = event;

    match payload {
        StreamEventPayload::SessionSnapshot { session } => apply_session_snapshot(session, signals),
        StreamEventPayload::ConversationMessage { message } => {
            apply_conversation_message(message, signals)
        }
        StreamEventPayload::PermissionRequested { request } => {
            apply_permission_request(request, signals)
        }
        StreamEventPayload::SessionClosed { reason, .. } => {
            apply_session_closed(sequence, reason, signals)
        }
        StreamEventPayload::Status { message } => apply_status_update(sequence, message, signals),
    }
}

fn apply_session_snapshot(session: SessionSnapshot, signals: StreamSignals) {
    let bootstrap = session_bootstrap_from_snapshot(session);
    signals.session_status.set(bootstrap.session_status);
    if should_apply_snapshot_turn_state(signals.turn_state.get_untracked()) {
        signals
            .turn_state
            .set(turn_state_for_snapshot(&bootstrap.pending_permissions));
    }
    signals
        .pending_permissions
        .set(bootstrap.pending_permissions);
    signals.entries.set(bootstrap.entries);
}

fn apply_conversation_message(message: ConversationMessage, signals: StreamSignals) {
    let is_assistant_message = matches!(message.role, MessageRole::Assistant);
    let mut appended = false;
    signals.entries.update(|current_entries| {
        if !current_entries.iter().any(|entry| entry.id == message.id) {
            appended = true;
            current_entries.push(message_to_entry(message));
        }
    });
    if appended
        && is_assistant_message
        && should_release_turn_state_for_assistant_message(signals.turn_state.get_untracked())
    {
        signals.turn_state.set(TurnState::Idle);
    }
}

fn apply_permission_request(request: PermissionRequest, signals: StreamSignals) {
    signals.pending_permissions.update(|current_permissions| {
        if !current_permissions
            .iter()
            .any(|(request_id, _)| request_id == &request.request_id)
        {
            current_permissions.push((request.request_id, request.summary));
        }
    });
    signals.turn_state.set(TurnState::AwaitingPermission);
}

fn apply_session_closed(sequence: u64, reason: String, signals: StreamSignals) {
    signals.session_status.set("closed".to_string());
    signals.turn_state.set(TurnState::Idle);
    push_status_entry(signals.entries, sequence, reason);
}

fn apply_status_update(sequence: u64, message: String, signals: StreamSignals) {
    if should_release_turn_state_for_status(signals.turn_state.get_untracked()) {
        signals.turn_state.set(TurnState::Idle);
    }
    push_status_entry(signals.entries, sequence, message);
}

fn session_bootstrap_from_snapshot(session: SessionSnapshot) -> SessionBootstrap {
    let SessionSnapshot {
        status,
        messages,
        pending_permissions,
        ..
    } = session;

    SessionBootstrap {
        entries: messages.into_iter().map(message_to_entry).collect(),
        pending_permissions: pending_permissions_to_items(pending_permissions),
        session_status: session_status_label(status).to_string(),
    }
}

async fn classify_session_load_failure(response: gloo_net::http::Response) -> SessionLoadError {
    let status = response.status();
    let backend_message = read_backend_error_message(response).await;
    match status {
        401 | 403 | 404 => SessionLoadError::ResumeUnavailable(session_unavailable_message(
            status,
            backend_message,
        )),
        _ => SessionLoadError::Other(format_api_failure(
            "Load session failed",
            status,
            backend_message,
        )),
    }
}

async fn response_error_message(response: gloo_net::http::Response, action: &str) -> String {
    let status = response.status();
    let backend_message = read_backend_error_message(response).await;
    format_api_failure(action, status, backend_message)
}

async fn read_backend_error_message(response: gloo_net::http::Response) -> Option<String> {
    let body = response.text().await.ok()?;
    decode_backend_error_message(&body)
}

fn decode_backend_error_message(body: &str) -> Option<String> {
    serde_json::from_str::<ErrorResponse>(body)
        .ok()
        .map(|response| response.error)
}

fn format_api_failure(action: &str, status: u16, backend_message: Option<String>) -> String {
    backend_message
        .map(|message| format!("{action}: {message}"))
        .unwrap_or_else(|| format!("{action}: HTTP {status}"))
}

fn session_unavailable_message(status: u16, backend_message: Option<String>) -> String {
    let detail = backend_message.unwrap_or_else(|| format!("HTTP {status}"));
    format!("This session is unavailable ({detail}). Start a fresh chat.")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_backend_error_message_reads_error_response() {
        let body = serde_json::json!({
            "error": "session not found"
        })
        .to_string();

        assert_eq!(
            decode_backend_error_message(&body),
            Some("session not found".to_string())
        );
    }

    #[test]
    fn session_unavailable_message_includes_backend_details() {
        assert_eq!(
            session_unavailable_message(404, Some("session not found".to_string())),
            "This session is unavailable (session not found). Start a fresh chat."
        );
    }

    #[test]
    fn session_bootstrap_from_snapshot_maps_messages_and_permissions() {
        let body = serde_json::json!({
            "session": {
                "id": "s_123",
                "status": "closed",
                "latest_sequence": 8,
                "messages": [
                    {
                        "id": "m_user",
                        "role": "user",
                        "text": "hello",
                        "created_at": "2026-04-17T01:00:00Z"
                    },
                    {
                        "id": "m_assistant",
                        "role": "assistant",
                        "text": "world",
                        "created_at": "2026-04-17T01:00:01Z"
                    }
                ],
                "pending_permissions": [{
                    "request_id": "req_1",
                    "summary": "read README.md"
                }]
            }
        })
        .to_string();

        let bootstrap = session_bootstrap_from_snapshot(
            serde_json::from_str::<SessionResponse>(&body)
                .expect("wrapped session payload should decode")
                .session,
        );

        assert_eq!(bootstrap.session_status, "closed");
        assert_eq!(bootstrap.entries.len(), 2);
        assert_eq!(bootstrap.entries[0].id, "m_user");
        assert!(matches!(bootstrap.entries[0].role, EntryRole::User));
        assert_eq!(bootstrap.entries[1].id, "m_assistant");
        assert!(matches!(bootstrap.entries[1].role, EntryRole::Assistant));
        assert_eq!(
            bootstrap.pending_permissions,
            vec![("req_1".to_string(), "read README.md".to_string())]
        );
    }
}
