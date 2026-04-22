use acp_contracts::ErrorResponse;

use super::errors::{
    SessionLoadError, classify_session_load_failure_parts, decode_backend_error_message,
    format_response_error_message,
};

pub(crate) async fn classify_session_load_failure(
    response: gloo_net::http::Response,
) -> SessionLoadError {
    classify_session_load_failure_parts(response.status(), read_backend_error_message(response).await)
}

pub(crate) async fn response_error_message(
    response: gloo_net::http::Response,
    fallback: &str,
) -> String {
    format_response_error_message(
        response.status(),
        fallback,
        response.json::<ErrorResponse>().await.ok(),
    )
}

async fn read_backend_error_message(response: gloo_net::http::Response) -> Option<String> {
    decode_backend_error_message(&response.text().await.ok()?)
}
