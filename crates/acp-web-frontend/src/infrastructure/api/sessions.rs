#![cfg_attr(not(target_family = "wasm"), allow(dead_code))]

use acp_contracts_messages::{PromptRequest};
use acp_contracts_permissions::{PermissionDecision, ResolvePermissionRequest};
use acp_contracts_sessions::{CancelTurnResponse, DeleteSessionResponse, RenameSessionRequest, SessionListItem, SessionSnapshot};
#[cfg(target_family = "wasm")]
use acp_contracts_sessions::{CreateSessionResponse, RenameSessionResponse, SessionListResponse, SessionResponse};
#[cfg(target_family = "wasm")]
use gloo_net::http::Request;

use super::{SessionLoadError, permission_url, session_path};
#[cfg(target_family = "wasm")]
use super::{classify_session_load_failure, response_error_message};
#[cfg(target_family = "wasm")]
use super::{csrf_token, patch_json_with_csrf, post_json_with_csrf};

const SESSIONS_URL: &str = "/api/v1/sessions";

#[cfg(target_family = "wasm")]
pub(crate) async fn create_session() -> Result<String, String> {
    let csrf = csrf_token();
    let response = Request::post(SESSIONS_URL)
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

#[cfg(not(target_family = "wasm"))]
pub(crate) async fn create_session() -> Result<String, String> {
    Err(non_wasm_session_error("POST", SESSIONS_URL))
}

#[cfg(target_family = "wasm")]
pub(crate) async fn load_session(session_id: &str) -> Result<SessionSnapshot, SessionLoadError> {
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

#[cfg(not(target_family = "wasm"))]
pub(crate) async fn load_session(session_id: &str) -> Result<SessionSnapshot, SessionLoadError> {
    Err(SessionLoadError::Other(non_wasm_session_error(
        "GET",
        &session_path(session_id),
    )))
}

#[cfg(target_family = "wasm")]
pub(crate) async fn list_sessions() -> Result<Vec<SessionListItem>, String> {
    let response = Request::get(SESSIONS_URL)
        .send()
        .await
        .map_err(|error| error.to_string())?;

    if !response.ok() {
        return Err(response_error_message(response, "List sessions failed").await);
    }

    let listed: SessionListResponse = response.json().await.map_err(|error| error.to_string())?;
    Ok(listed.sessions)
}

#[cfg(not(target_family = "wasm"))]
pub(crate) async fn list_sessions() -> Result<Vec<SessionListItem>, String> {
    Err(non_wasm_session_error("GET", SESSIONS_URL))
}

#[cfg(target_family = "wasm")]
pub(crate) async fn send_message(session_id: &str, text: &str) -> Result<(), String> {
    let response =
        post_json_with_csrf(&session_messages_url(session_id), send_message_body(text)?).await?;

    if !response.ok() {
        return Err(response_error_message(response, "Send message failed").await);
    }
    Ok(())
}

#[cfg(not(target_family = "wasm"))]
pub(crate) async fn send_message(session_id: &str, text: &str) -> Result<(), String> {
    let _ = send_message_body(text)?;
    Err(non_wasm_session_error(
        "POST",
        &session_messages_url(session_id),
    ))
}

#[cfg(target_family = "wasm")]
pub(crate) async fn resolve_permission(
    session_id: &str,
    request_id: &str,
    decision: PermissionDecision,
) -> Result<(), String> {
    let url = permission_url(session_id, request_id);
    let response = post_json_with_csrf(&url, resolve_permission_body(decision)?).await?;

    if !response.ok() {
        return Err(response_error_message(response, "Resolve permission failed").await);
    }

    Ok(())
}

#[cfg(not(target_family = "wasm"))]
pub(crate) async fn resolve_permission(
    session_id: &str,
    request_id: &str,
    decision: PermissionDecision,
) -> Result<(), String> {
    let _ = resolve_permission_body(decision)?;
    Err(non_wasm_session_error(
        "POST",
        &permission_url(session_id, request_id),
    ))
}

#[cfg(target_family = "wasm")]
pub(crate) async fn cancel_turn(session_id: &str) -> Result<CancelTurnResponse, String> {
    let csrf = csrf_token();
    let response = Request::post(&cancel_turn_url(session_id))
        .header("x-csrf-token", &csrf)
        .send()
        .await
        .map_err(|error| error.to_string())?;

    if !response.ok() {
        return Err(response_error_message(response, "Cancel turn failed").await);
    }

    response.json().await.map_err(|error| error.to_string())
}

#[cfg(not(target_family = "wasm"))]
pub(crate) async fn cancel_turn(session_id: &str) -> Result<CancelTurnResponse, String> {
    Err(non_wasm_session_error("POST", &cancel_turn_url(session_id)))
}

#[cfg(target_family = "wasm")]
pub(crate) async fn rename_session(
    session_id: &str,
    title: &str,
) -> Result<SessionSnapshot, String> {
    let url = session_path(session_id);
    let response = patch_json_with_csrf(&url, rename_session_body(title)?).await?;

    if !response.ok() {
        return Err(response_error_message(response, "Rename session failed").await);
    }

    let renamed: RenameSessionResponse =
        response.json().await.map_err(|error| error.to_string())?;
    Ok(renamed.session)
}

#[cfg(not(target_family = "wasm"))]
pub(crate) async fn rename_session(
    session_id: &str,
    title: &str,
) -> Result<SessionSnapshot, String> {
    let _ = rename_session_body(title)?;
    Err(non_wasm_session_error("PATCH", &session_path(session_id)))
}

#[cfg(target_family = "wasm")]
pub(crate) async fn delete_session(session_id: &str) -> Result<DeleteSessionResponse, String> {
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

#[cfg(not(target_family = "wasm"))]
pub(crate) async fn delete_session(session_id: &str) -> Result<DeleteSessionResponse, String> {
    Err(non_wasm_session_error("DELETE", &session_path(session_id)))
}

fn session_messages_url(session_id: &str) -> String {
    format!("{}/messages", session_path(session_id))
}

fn cancel_turn_url(session_id: &str) -> String {
    format!("{}/cancel", session_path(session_id))
}

fn send_message_body(text: &str) -> Result<String, String> {
    serde_json::to_string(&PromptRequest {
        text: text.to_string(),
    })
    .map_err(|error| error.to_string())
}

fn resolve_permission_body(decision: PermissionDecision) -> Result<String, String> {
    serde_json::to_string(&ResolvePermissionRequest { decision }).map_err(|error| error.to_string())
}

fn rename_session_body(title: &str) -> Result<String, String> {
    serde_json::to_string(&RenameSessionRequest {
        title: title.to_string(),
    })
    .map_err(|error| error.to_string())
}

fn non_wasm_session_error(method: &str, url: &str) -> String {
    format!("Browser {method} sessions API is unavailable on non-wasm targets: {url}")
}

#[cfg(test)]
mod tests {
    use crate::infrastructure::api::poll_ready;

    use super::*;

    #[test]
    fn session_request_urls_encode_session_ids() {
        assert_eq!(SESSIONS_URL, "/api/v1/sessions");
        assert_eq!(
            session_messages_url("s/1"),
            "/api/v1/sessions/s%2F1/messages"
        );
        assert_eq!(cancel_turn_url("s/1"), "/api/v1/sessions/s%2F1/cancel");
    }

    #[test]
    fn session_request_bodies_serialize_expected_payloads() {
        assert_eq!(
            send_message_body("hello").expect("message body"),
            r#"{"text":"hello"}"#
        );
        assert_eq!(
            resolve_permission_body(PermissionDecision::Approve).expect("permission body"),
            r#"{"decision":"approve"}"#
        );
        assert_eq!(
            rename_session_body("Rename me").expect("rename body"),
            r#"{"title":"Rename me"}"#
        );
    }

    #[test]
    fn host_session_api_functions_fail_with_descriptive_errors() {
        let create_error = poll_ready(create_session()).expect_err("host create should fail");
        assert!(create_error.contains(SESSIONS_URL));

        let load_error = poll_ready(load_session("s/1")).expect_err("host load should fail");
        assert!(matches!(
            load_error,
            SessionLoadError::Other(message) if message.contains("/api/v1/sessions/s%2F1")
        ));

        let list_error = poll_ready(list_sessions()).expect_err("host list should fail");
        assert!(list_error.contains(SESSIONS_URL));

        let send_error =
            poll_ready(send_message("s/1", "hello")).expect_err("host send should fail");
        assert!(send_error.contains("/api/v1/sessions/s%2F1/messages"));

        let permission_error = poll_ready(resolve_permission(
            "s/1",
            "req 1",
            PermissionDecision::Approve,
        ))
        .expect_err("host permission should fail");
        assert!(permission_error.contains("/api/v1/sessions/s%2F1/permissions/req%201"));

        let cancel_error = poll_ready(cancel_turn("s/1")).expect_err("host cancel should fail");
        assert!(cancel_error.contains("/api/v1/sessions/s%2F1/cancel"));

        let rename_error =
            poll_ready(rename_session("s/1", "Renamed")).expect_err("host rename should fail");
        assert!(rename_error.contains("/api/v1/sessions/s%2F1"));

        let delete_error = poll_ready(delete_session("s/1")).expect_err("host delete should fail");
        assert!(delete_error.contains("/api/v1/sessions/s%2F1"));
    }
}
