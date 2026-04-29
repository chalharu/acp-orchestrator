use axum::{
    Json,
    extract::{Extension, Path, State},
    http::{HeaderMap, header},
};
use serde::de::DeserializeOwned;

use crate::auth::AuthenticatedPrincipal;
use crate::contract_messages::{PromptRequest, PromptResponse};
use crate::contract_permissions::{ResolvePermissionRequest, ResolvePermissionResponse};
use crate::contract_sessions::{
    CancelTurnResponse, CloseSessionResponse, DeleteSessionResponse, RenameSessionRequest,
    RenameSessionResponse,
};
#[cfg(test)]
use crate::contract_sessions::{CreateSessionRequest, CreateSessionResponse};

#[cfg(test)]
use super::super::session_service::create_session_snapshot;
use super::super::{
    AppError, AppState,
    session_service::{
        close_live_session, delete_live_session, rename_session_title, submit_prompt,
    },
};

#[cfg(test)]
pub(in crate::server) async fn create_session(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
    body: axum::body::Bytes,
) -> Result<(axum::http::StatusCode, Json<CreateSessionResponse>), AppError> {
    let request = parse_optional_json_body::<CreateSessionRequest>(&body)?.unwrap_or_default();
    let session = create_session_snapshot(&state, principal, request.checkout_ref).await?;

    Ok((
        axum::http::StatusCode::CREATED,
        Json(CreateSessionResponse { session }),
    ))
}

pub(in crate::server) fn parse_optional_json_body<T>(body: &[u8]) -> Result<Option<T>, AppError>
where
    T: DeserializeOwned,
{
    if body.iter().all(|byte| byte.is_ascii_whitespace()) {
        return Ok(None);
    }

    serde_json::from_slice(body)
        .map(Some)
        .map_err(|error| AppError::BadRequest(format!("invalid request body: {error}")))
}

pub(in crate::server) fn parse_json_body<T>(headers: &HeaderMap, body: &[u8]) -> Result<T, AppError>
where
    T: DeserializeOwned,
{
    require_json_content_type(headers)?;
    parse_optional_json_body(body)?
        .ok_or_else(|| AppError::BadRequest("request body is required".to_string()))
}

fn require_json_content_type(headers: &HeaderMap) -> Result<(), AppError> {
    if json_content_type(headers) {
        Ok(())
    } else {
        Err(AppError::UnsupportedMediaType(
            "expected request with `Content-Type: application/json`".to_string(),
        ))
    }
}

fn json_content_type(headers: &HeaderMap) -> bool {
    let Some(content_type) = headers.get(header::CONTENT_TYPE) else {
        return false;
    };
    let Ok(content_type) = content_type.to_str() else {
        return false;
    };
    let media_type = content_type.split(';').next().unwrap_or_default().trim();
    let Some((top_level, subtype)) = media_type.split_once('/') else {
        return false;
    };

    top_level.eq_ignore_ascii_case("application")
        && (subtype.eq_ignore_ascii_case("json")
            || subtype
                .rsplit_once('+')
                .is_some_and(|(_, suffix)| suffix.eq_ignore_ascii_case("json")))
}

pub(in crate::server) async fn rename_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
    Json(request): Json<RenameSessionRequest>,
) -> Result<Json<RenameSessionResponse>, AppError> {
    let session = rename_session_title(&state, principal, &session_id, request.title).await?;

    Ok(Json(RenameSessionResponse { session }))
}

pub(in crate::server) async fn delete_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
) -> Result<Json<DeleteSessionResponse>, AppError> {
    delete_live_session(&state, principal, &session_id).await?;

    Ok(Json(DeleteSessionResponse { deleted: true }))
}

pub(in crate::server) async fn post_message(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
    Json(request): Json<PromptRequest>,
) -> Result<Json<PromptResponse>, AppError> {
    submit_prompt(&state, principal, &session_id, request.text).await?;

    Ok(Json(PromptResponse { accepted: true }))
}

pub(in crate::server) async fn close_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
) -> Result<Json<CloseSessionResponse>, AppError> {
    let session = close_live_session(&state, principal, &session_id).await?;

    Ok(Json(CloseSessionResponse { session }))
}

pub(in crate::server) async fn cancel_turn(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
) -> Result<Json<CancelTurnResponse>, AppError> {
    let owner = state.owner_context(principal).await?;
    let cancelled = state
        .store
        .cancel_active_turn(&owner.principal.id, &session_id)
        .await?;

    Ok(Json(CancelTurnResponse { cancelled }))
}

pub(in crate::server) async fn resolve_permission(
    State(state): State<AppState>,
    Path((session_id, request_id)): Path<(String, String)>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
    Json(request): Json<ResolvePermissionRequest>,
) -> Result<Json<ResolvePermissionResponse>, AppError> {
    let owner = state.owner_context(principal).await?;
    let resolution = state
        .store
        .resolve_permission(
            &owner.principal.id,
            &session_id,
            &request_id,
            request.decision,
        )
        .await?;

    Ok(Json(resolution))
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;
    use serde::Deserialize;

    use super::*;

    #[derive(Debug, Deserialize, PartialEq)]
    struct RequestBody {
        value: String,
    }

    fn json_headers(content_type: Option<&str>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Some(content_type) = content_type {
            headers.insert(
                header::CONTENT_TYPE,
                HeaderValue::from_str(content_type).expect("valid content type"),
            );
        }
        headers
    }

    #[test]
    fn parse_json_body_requires_json_content_type() {
        let body = br#"{"value":"ok"}"#;
        for content_type in [None, Some("text/json"), Some("text/plain"), Some("json")] {
            let error = parse_json_body::<RequestBody>(&json_headers(content_type), body)
                .expect_err("non-json content type should fail");
            assert!(matches!(error, AppError::UnsupportedMediaType(_)));
        }

        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_bytes(b"application/\xff").expect("opaque header bytes are allowed"),
        );
        let error = parse_json_body::<RequestBody>(&headers, body)
            .expect_err("non-UTF-8 content type should fail");
        assert!(matches!(error, AppError::UnsupportedMediaType(_)));
    }

    #[test]
    fn parse_json_body_accepts_json_and_json_suffix_content_types() {
        for content_type in [
            "application/json",
            "application/json; charset=utf-8",
            "application/cloudevents+json",
        ] {
            let parsed = parse_json_body::<RequestBody>(
                &json_headers(Some(content_type)),
                br#"{"value":"ok"}"#,
            )
            .expect("json content type should parse");
            assert_eq!(
                parsed,
                RequestBody {
                    value: "ok".to_string()
                }
            );
        }
    }
}
