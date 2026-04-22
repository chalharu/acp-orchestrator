use acp_contracts::{
    CancelTurnResponse, CreateSessionResponse, DeleteSessionResponse, PermissionDecision,
    PromptRequest, RenameSessionRequest, RenameSessionResponse, ResolvePermissionRequest,
    SessionListItem, SessionListResponse, SessionResponse, SessionSnapshot,
};
use gloo_net::http::Request;

use super::{
    SessionLoadError, classify_session_load_failure, csrf_token, patch_json_with_csrf,
    permission_url, post_json_with_csrf, response_error_message, session_path,
};

const SESSIONS_URL: &str = "/api/v1/sessions";

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

pub(crate) async fn send_message(session_id: &str, text: &str) -> Result<(), String> {
    let response =
        post_json_with_csrf(&session_messages_url(session_id), send_message_body(text)?).await?;

    if !response.ok() {
        return Err(response_error_message(response, "Send message failed").await);
    }
    Ok(())
}

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

#[cfg(test)]
mod tests {
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
}
