use std::{future::Future, time::Duration};

use acp_contracts::{AssistantReplyRequest, AssistantReplyResponse, HealthResponse};
use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use tokio::{net::TcpListener, time::sleep};
use tracing::info;

#[derive(Debug, Clone)]
pub struct MockConfig {
    pub response_delay: Duration,
}

impl Default for MockConfig {
    fn default() -> Self {
        Self {
            response_delay: Duration::from_millis(120),
        }
    }
}

#[derive(Debug, Clone)]
struct MockState {
    config: MockConfig,
}

pub fn app(config: MockConfig) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/v1/reply", post(reply))
        .with_state(MockState { config })
}

pub async fn serve_with_shutdown<F>(
    listener: TcpListener,
    config: MockConfig,
    shutdown: F,
) -> std::io::Result<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    let address = listener.local_addr()?;
    info!("starting acp mock on {address}");
    axum::serve(listener, app(config))
        .with_graceful_shutdown(shutdown)
        .await
}

async fn healthz() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
    })
}

async fn reply(
    State(state): State<MockState>,
    Json(request): Json<AssistantReplyRequest>,
) -> Json<AssistantReplyResponse> {
    sleep(state.config.response_delay).await;

    Json(AssistantReplyResponse {
        text: reply_for(&request.prompt),
    })
}

pub fn reply_for(prompt: &str) -> String {
    let compact = prompt.split_whitespace().collect::<Vec<_>>().join(" ");

    format!(
        "mock assistant: I received `{}`. This flow still uses a mock ACP worker, but the backend-to-mock round-trip succeeded.",
        truncate(&compact, 120)
    )
}

fn truncate(value: &str, max_len: usize) -> String {
    let mut chars = value.chars();
    let truncated = chars.by_ref().take(max_len).collect::<String>();

    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_uses_the_expected_delay() {
        assert_eq!(
            MockConfig::default().response_delay,
            Duration::from_millis(120)
        );
    }

    #[test]
    fn long_prompts_are_truncated_in_mock_replies() {
        let prompt = "word ".repeat(80);
        let reply = reply_for(&prompt);

        assert!(reply.contains("...`"));
        assert!(reply.starts_with("mock assistant: I received `"));
    }
}
