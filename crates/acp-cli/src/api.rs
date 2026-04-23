use super::*;
use crate::contract_sessions::{
    CreateSessionResponse, SessionHistoryResponse, SessionListResponse, SessionResponse,
    SessionSnapshot,
};
use crate::contract_slash::SlashCompletionsResponse;

pub(super) async fn list_sessions(
    client: &Client,
    base_url: &str,
    auth_token: &str,
) -> Result<SessionListResponse> {
    let response = client
        .get(format!("{base_url}/api/v1/sessions"))
        .bearer_auth(auth_token)
        .send()
        .await
        .context(SendRequestSnafu {
            action: "list sessions",
        })?;
    let response = ensure_success(response, "list sessions").await?;
    response.json().await.context(DecodeResponseSnafu {
        action: "list sessions",
    })
}

pub(super) async fn create_session(
    client: &Client,
    base_url: &str,
    auth_token: &str,
) -> Result<SessionSnapshot> {
    let response = client
        .post(format!("{base_url}/api/v1/sessions"))
        .bearer_auth(auth_token)
        .send()
        .await
        .context(SendRequestSnafu {
            action: "create session",
        })?;
    let response = ensure_success(response, "create session").await?;
    let payload: CreateSessionResponse = response.json().await.context(DecodeResponseSnafu {
        action: "create session",
    })?;
    Ok(payload.session)
}

pub(super) async fn get_session(
    client: &Client,
    base_url: &str,
    auth_token: &str,
    session_id: &str,
) -> Result<SessionSnapshot> {
    let response = client
        .get(format!("{base_url}/api/v1/sessions/{session_id}"))
        .bearer_auth(auth_token)
        .send()
        .await
        .context(SendRequestSnafu {
            action: "load session",
        })?;
    let response = ensure_success(response, "load session").await?;
    let payload: SessionResponse = response.json().await.context(DecodeResponseSnafu {
        action: "load session",
    })?;
    Ok(payload.session)
}

pub(super) async fn get_session_history(
    client: &Client,
    base_url: &str,
    auth_token: &str,
    session_id: &str,
) -> Result<SessionHistoryResponse> {
    let response = client
        .get(format!("{base_url}/api/v1/sessions/{session_id}/history"))
        .bearer_auth(auth_token)
        .send()
        .await
        .context(SendRequestSnafu {
            action: "load session history",
        })?;
    let response = ensure_success(response, "load session history").await?;
    response.json().await.context(DecodeResponseSnafu {
        action: "load session history",
    })
}

pub(super) async fn submit_prompt(
    client: &Client,
    base_url: &str,
    auth_token: &str,
    session_id: &str,
    prompt: &str,
) -> Result<()> {
    let response = client
        .post(format!("{base_url}/api/v1/sessions/{session_id}/messages"))
        .bearer_auth(auth_token)
        .json(&PromptRequest {
            text: prompt.to_string(),
        })
        .send()
        .await
        .context(SendRequestSnafu {
            action: "submit prompt",
        })?;
    let response = ensure_success(response, "submit prompt").await?;
    let _: PromptResponse = response.json().await.context(DecodeResponseSnafu {
        action: "submit prompt",
    })?;
    Ok(())
}

pub(super) async fn close_session(
    client: &Client,
    base_url: &str,
    auth_token: &str,
    session_id: &str,
) -> Result<CloseSessionResponse> {
    let response = client
        .post(format!("{base_url}/api/v1/sessions/{session_id}/close"))
        .bearer_auth(auth_token)
        .send()
        .await
        .context(SendRequestSnafu {
            action: "close session",
        })?;
    let response = ensure_success(response, "close session").await?;
    response.json().await.context(DecodeResponseSnafu {
        action: "close session",
    })
}

pub(super) async fn cancel_turn(
    client: &Client,
    base_url: &str,
    auth_token: &str,
    session_id: &str,
) -> Result<CancelTurnResponse> {
    let response = client
        .post(format!("{base_url}/api/v1/sessions/{session_id}/cancel"))
        .bearer_auth(auth_token)
        .send()
        .await
        .context(SendRequestSnafu {
            action: "cancel turn",
        })?;
    let response = ensure_success(response, "cancel turn").await?;
    response.json().await.context(DecodeResponseSnafu {
        action: "cancel turn",
    })
}

pub(super) async fn resolve_permission(
    client: &Client,
    base_url: &str,
    auth_token: &str,
    session_id: &str,
    request_id: &str,
    decision: PermissionDecision,
) -> Result<ResolvePermissionResponse> {
    let response = client
        .post(format!(
            "{base_url}/api/v1/sessions/{session_id}/permissions/{request_id}"
        ))
        .bearer_auth(auth_token)
        .json(&ResolvePermissionRequest { decision })
        .send()
        .await
        .context(SendRequestSnafu {
            action: "resolve permission",
        })?;
    let response = ensure_success(response, "resolve permission").await?;
    response.json().await.context(DecodeResponseSnafu {
        action: "resolve permission",
    })
}

pub(super) async fn get_slash_completions(
    client: &Client,
    base_url: &str,
    auth_token: &str,
    session_id: &str,
    prefix: &str,
) -> Result<SlashCompletionsResponse> {
    let mut completions_url = reqwest::Url::parse(&format!("{base_url}/api/v1/completions/slash"))
        .expect("server URLs should be valid before querying slash completions");
    completions_url
        .query_pairs_mut()
        .append_pair("sessionId", session_id)
        .append_pair("prefix", prefix);
    let response = client
        .get(completions_url)
        .bearer_auth(auth_token)
        .send()
        .await
        .context(SendRequestSnafu {
            action: "load slash completions",
        })?;
    let response = ensure_success(response, "load slash completions").await?;
    response.json().await.context(DecodeResponseSnafu {
        action: "load slash completions",
    })
}

pub(super) async fn ensure_success(response: Response, action: &'static str) -> Result<Response> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }

    let message = match response.json::<ErrorResponse>().await {
        Ok(payload) => payload.error,
        Err(_) => status
            .canonical_reason()
            .unwrap_or("request failed")
            .to_string(),
    };

    HttpStatusSnafu {
        action,
        status,
        message,
    }
    .fail()
}
