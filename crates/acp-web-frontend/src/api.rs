//! Thin async wrappers over the ACP backend REST/SSE API.
//!
//! The frontend reuses `acp-contracts` directly so the browser and backend stay
//! on the same wire schema for session snapshots, prompt submission, and named
//! SSE events.

use acp_contracts::{
    CancelTurnResponse, CreateSessionResponse, DeleteSessionResponse, ErrorResponse,
    PermissionDecision, PromptRequest, RenameSessionRequest, RenameSessionResponse,
    ResolvePermissionRequest, SessionListItem, SessionListResponse, SessionResponse,
    SessionSnapshot, StreamEvent,
};
use futures_channel::mpsc;
use gloo_net::http::Request;
use wasm_bindgen::{JsCast, closure::Closure};
use web_sys::{Event, EventSource, MessageEvent};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionLoadError {
    ResumeUnavailable(String),
    Other(String),
}

pub enum SseItem {
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
pub async fn load_session(session_id: &str) -> Result<SessionSnapshot, SessionLoadError> {
    let url = session_path(session_id);
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

    Ok(session.session)
}

/// Load the current user's sessions in backend-provided order.
pub async fn list_sessions() -> Result<Vec<SessionListItem>, String> {
    let response = Request::get("/api/v1/sessions")
        .send()
        .await
        .map_err(|error| error.to_string())?;

    if !response.ok() {
        return Err(response_error_message(response, "List sessions failed").await);
    }

    let listed: SessionListResponse = response.json().await.map_err(|error| error.to_string())?;
    Ok(listed.sessions)
}

/// Open the session event stream and return the raw `EventSource` plus a parsed
/// event receiver.
pub fn open_session_event_stream(
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

/// POST a new message to an existing session.
pub async fn send_message(session_id: &str, text: &str) -> Result<(), String> {
    let url = format!("{}/messages", session_path(session_id));
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
    let url = permission_url(session_id, request_id);
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
    let url = format!("{}/cancel", session_path(session_id));
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

/// PATCH the session title.
pub async fn rename_session(session_id: &str, title: &str) -> Result<SessionSnapshot, String> {
    let url = session_path(session_id);
    let body = serde_json::to_string(&RenameSessionRequest {
        title: title.to_string(),
    })
    .map_err(|error| error.to_string())?;

    let response = patch_json_with_csrf(&url, body).await?;

    if !response.ok() {
        return Err(response_error_message(response, "Rename session failed").await);
    }

    let renamed: RenameSessionResponse =
        response.json().await.map_err(|error| error.to_string())?;
    Ok(renamed.session)
}

/// DELETE a session permanently.
pub async fn delete_session(session_id: &str) -> Result<DeleteSessionResponse, String> {
    let csrf = csrf_token();
    let url = session_path(session_id);
    let response = Request::delete(&url)
        .header("x-csrf-token", &csrf)
        .send()
        .await
        .map_err(|error| error.to_string())?;

    if !response.ok() {
        return Err(response_error_message(response, "Delete session failed").await);
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

async fn patch_json_with_csrf(url: &str, body: String) -> Result<gloo_net::http::Response, String> {
    let csrf = csrf_token();
    Request::patch(url)
        .header("x-csrf-token", &csrf)
        .header("content-type", "application/json")
        .body(body)
        .map_err(|error| error.to_string())?
        .send()
        .await
        .map_err(|error| error.to_string())
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

fn session_path(session_id: &str) -> String {
    format!("/api/v1/sessions/{}", encode_component(session_id))
}

fn permission_url(session_id: &str, request_id: &str) -> String {
    format!(
        "{}/permissions/{}",
        session_path(session_id),
        encode_component(request_id),
    )
}

pub(crate) fn encode_component(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push('%');
            encoded.push_str(&format!("{byte:02X}"));
        }
    }
    encoded
}

pub(crate) fn decode_component(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] != b'%' {
            decoded.push(bytes[index]);
            index += 1;
            continue;
        }

        let high = *bytes.get(index + 1)?;
        let low = *bytes.get(index + 2)?;
        decoded.push((hex_value(high)? << 4) | hex_value(low)?);
        index += 3;
    }

    String::from_utf8(decoded).ok()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn session_unavailable_message(status: u16, backend_message: Option<String>) -> String {
    let detail = backend_message.unwrap_or_else(|| format!("HTTP {status}"));
    format!("This session is unavailable ({detail}). Start a fresh chat.")
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
    fn session_path_encodes_special_characters() {
        assert_eq!(session_path("s_123"), "/api/v1/sessions/s_123");
        assert_eq!(session_path("s/1"), "/api/v1/sessions/s%2F1");
        assert_eq!(session_path("../../etc"), "/api/v1/sessions/..%2F..%2Fetc");
    }

    #[test]
    fn decode_component_decodes_percent_encoded_utf8() {
        assert_eq!(decode_component("s%2F1"), Some("s/1".to_string()));
        assert_eq!(
            decode_component("hello%20world"),
            Some("hello world".to_string())
        );
        assert_eq!(decode_component("%E3%81%82"), Some("あ".to_string()));
        assert_eq!(decode_component("%ZZ"), None);
        assert_eq!(decode_component("%A"), None);
    }

    #[test]
    fn permission_url_encodes_session_and_request_ids() {
        assert_eq!(
            permission_url("s/1", "../../close"),
            "/api/v1/sessions/s%2F1/permissions/..%2F..%2Fclose"
        );
    }
}
