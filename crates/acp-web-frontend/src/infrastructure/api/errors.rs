#![cfg_attr(not(any(test, target_family = "wasm")), allow(dead_code))]

use acp_contracts::ErrorResponse;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SessionLoadError {
    ResumeUnavailable(String),
    Other(String),
}

pub(crate) fn classify_session_load_failure_parts(
    status: u16,
    backend_message: Option<String>,
) -> SessionLoadError {
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

pub(crate) fn format_response_error_message(
    status: u16,
    fallback: &str,
    error: Option<ErrorResponse>,
) -> String {
    match error {
        Some(error) if !error.error.trim().is_empty() => error.error,
        _ => format!("{fallback} (status {status})"),
    }
}

pub(crate) fn decode_backend_error_message(body: &str) -> Option<String> {
    serde_json::from_str::<ErrorResponse>(body)
        .ok()
        .map(|response| response.error)
}

pub(crate) fn format_api_failure(
    action: &str,
    status: u16,
    backend_message: Option<String>,
) -> String {
    backend_message
        .map(|message| format!("{action}: {message}"))
        .unwrap_or_else(|| format!("{action}: HTTP {status}"))
}

pub(crate) fn session_unavailable_message(status: u16, backend_message: Option<String>) -> String {
    let detail = backend_message.unwrap_or_else(|| format!("HTTP {status}"));
    format!("This session is unavailable ({detail}). Start a fresh chat.")
}

#[cfg(test)]
mod tests {
    use acp_contracts::ErrorResponse;

    use super::{
        SessionLoadError, classify_session_load_failure_parts, decode_backend_error_message,
        format_api_failure, format_response_error_message, session_unavailable_message,
    };

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
    fn decode_backend_error_message_rejects_invalid_payloads() {
        assert_eq!(decode_backend_error_message("not json"), None);
    }

    #[test]
    fn classify_session_load_failure_treats_resume_statuses_as_unavailable() {
        assert_eq!(
            classify_session_load_failure_parts(401, Some("sign in again".to_string())),
            SessionLoadError::ResumeUnavailable(
                "This session is unavailable (sign in again). Start a fresh chat.".to_string()
            )
        );
        assert_eq!(
            classify_session_load_failure_parts(403, None),
            SessionLoadError::ResumeUnavailable(
                "This session is unavailable (HTTP 403). Start a fresh chat.".to_string()
            )
        );
        assert_eq!(
            classify_session_load_failure_parts(404, Some("session not found".to_string())),
            SessionLoadError::ResumeUnavailable(
                "This session is unavailable (session not found). Start a fresh chat.".to_string()
            )
        );
    }

    #[test]
    fn classify_session_load_failure_formats_other_statuses_as_api_failures() {
        assert_eq!(
            classify_session_load_failure_parts(500, Some("boom".to_string())),
            SessionLoadError::Other("Load session failed: boom".to_string())
        );
        assert_eq!(
            classify_session_load_failure_parts(502, None),
            SessionLoadError::Other("Load session failed: HTTP 502".to_string())
        );
    }

    #[test]
    fn format_response_error_message_uses_backend_message_only_when_present() {
        assert_eq!(
            format_response_error_message(
                409,
                "Create account failed",
                Some(ErrorResponse {
                    error: "name taken".to_string(),
                }),
            ),
            "name taken"
        );
        assert_eq!(
            format_response_error_message(
                409,
                "Create account failed",
                Some(ErrorResponse {
                    error: "   ".to_string(),
                }),
            ),
            "Create account failed (status 409)"
        );
        assert_eq!(
            format_response_error_message(503, "Load accounts failed", None),
            "Load accounts failed (status 503)"
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
    fn session_unavailable_message_falls_back_to_http_status() {
        assert_eq!(
            session_unavailable_message(503, None),
            "This session is unavailable (HTTP 503). Start a fresh chat."
        );
    }

    #[test]
    fn format_api_failure_prefers_backend_message_and_has_status_fallback() {
        assert_eq!(
            format_api_failure("Load session failed", 500, Some("boom".to_string())),
            "Load session failed: boom"
        );
        assert_eq!(
            format_api_failure("Load session failed", 500, None),
            "Load session failed: HTTP 500"
        );
    }
}
