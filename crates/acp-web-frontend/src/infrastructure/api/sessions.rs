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

pub(crate) async fn create_session() -> Result<String, String> {
    let csrf = csrf_token();
    let response = Request::post("/api/v1/sessions")
        .header("x-csrf-token", &csrf)
        .send()
        .await
        .map_err(|error| error.to_string())?;

    if !response.ok() {
        return Err(response_error_message(response, "Create session failed").await);
    }

    let created: CreateSessionResponse = response.json().await.map_err(|error| error.to_string())?;
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

pub(crate) async fn send_message(session_id: &str, text: &str) -> Result<(), String> {
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

pub(crate) async fn resolve_permission(
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

pub(crate) async fn cancel_turn(session_id: &str) -> Result<CancelTurnResponse, String> {
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

pub(crate) async fn rename_session(
    session_id: &str,
    title: &str,
) -> Result<SessionSnapshot, String> {
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
