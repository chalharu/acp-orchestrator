use std::time::Duration;

use std::{future::Future, pin::Pin};

use acp_contracts::{AssistantReplyRequest, AssistantReplyResponse, ErrorResponse};
use reqwest::{Client, StatusCode};
use snafu::prelude::*;

type Result<T, E = MockClientError> = std::result::Result<T, E>;
pub type ReplyFuture<'a> = Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>>;
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

pub trait ReplyProvider: Send + Sync + std::fmt::Debug {
    fn request_reply<'a>(&'a self, session_id: &'a str, prompt: &'a str) -> ReplyFuture<'a>;
}

#[derive(Debug, Clone)]
pub struct MockClient {
    base_url: String,
    client: Client,
}

#[derive(Debug, Snafu)]
pub enum MockClientError {
    #[snafu(display("building the mock HTTP client failed"))]
    BuildHttpClient { source: reqwest::Error },

    #[snafu(display("sending the mock reply request failed"))]
    SendRequest { source: reqwest::Error },

    #[snafu(display("the mock service returned HTTP {status}: {message}"))]
    HttpStatus { status: StatusCode, message: String },

    #[snafu(display("decoding the mock reply failed"))]
    DecodeResponse { source: reqwest::Error },
}

impl MockClient {
    pub fn new(base_url: String) -> Result<Self> {
        Self::with_timeout(base_url, DEFAULT_REQUEST_TIMEOUT)
    }

    fn with_timeout(base_url: String, request_timeout: Duration) -> Result<Self> {
        let client = Client::builder()
            .timeout(request_timeout)
            .build()
            .context(BuildHttpClientSnafu)?;

        Ok(Self { base_url, client })
    }

    pub async fn request_reply(&self, session_id: &str, prompt: &str) -> Result<String> {
        let response = self
            .client
            .post(format!("{}/v1/reply", self.base_url))
            .json(&AssistantReplyRequest {
                session_id: session_id.to_string(),
                prompt: prompt.to_string(),
            })
            .send()
            .await
            .context(SendRequestSnafu)?;
        let response = ensure_success(response).await?;
        let payload: AssistantReplyResponse = response.json().await.context(DecodeResponseSnafu)?;
        Ok(payload.text)
    }
}

impl ReplyProvider for MockClient {
    fn request_reply<'a>(&'a self, session_id: &'a str, prompt: &'a str) -> ReplyFuture<'a> {
        Box::pin(MockClient::request_reply(self, session_id, prompt))
    }
}

async fn ensure_success(response: reqwest::Response) -> Result<reqwest::Response> {
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

    HttpStatusSnafu { status, message }.fail()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        http::{StatusCode, header::CONTENT_TYPE},
        routing::post,
    };
    use tokio::{net::TcpListener, sync::oneshot, task::JoinHandle};

    #[tokio::test]
    async fn request_reply_surfaces_json_error_messages() {
        let (base_url, shutdown_tx) = spawn_error_server(
            StatusCode::BAD_GATEWAY,
            "application/json",
            r#"{"error":"mock backend unavailable"}"#,
        )
        .await;
        let client = MockClient::new(base_url).expect("client construction should succeed");

        let error = client
            .request_reply("s_test", "hello")
            .await
            .expect_err("error response should fail");

        assert!(matches!(
            error,
            MockClientError::HttpStatus { status, message }
                if status == StatusCode::BAD_GATEWAY && message == "mock backend unavailable"
        ));

        let _ = shutdown_tx.shutdown.send(());
        shutdown_tx
            .handle
            .await
            .expect("test server task should join");
    }

    #[tokio::test]
    async fn request_reply_times_out_for_stalled_servers() {
        let (base_url, shutdown_tx) = spawn_delayed_server(Duration::from_millis(200)).await;
        let client = MockClient::with_timeout(base_url, Duration::from_millis(20))
            .expect("client construction should succeed");

        let error = client
            .request_reply("s_test", "hello")
            .await
            .expect_err("stalled responses should time out");

        assert!(matches!(error, MockClientError::SendRequest { .. }));

        let _ = shutdown_tx.shutdown.send(());
        shutdown_tx
            .handle
            .await
            .expect("test server task should join");
    }

    #[tokio::test]
    async fn request_reply_succeeds_when_the_server_recovers_before_timeout() {
        let (base_url, shutdown_tx) = spawn_delayed_server(Duration::from_millis(20)).await;
        let client = MockClient::with_timeout(base_url, Duration::from_millis(200))
            .expect("client construction should succeed");

        let reply = client
            .request_reply("s_test", "hello")
            .await
            .expect("responses within the timeout should succeed");

        assert_eq!(reply, "late");

        let _ = shutdown_tx.shutdown.send(());
        shutdown_tx
            .handle
            .await
            .expect("test server task should join");
    }

    #[tokio::test]
    async fn request_reply_falls_back_to_http_reason_for_non_json_errors() {
        let (base_url, shutdown_tx) =
            spawn_error_server(StatusCode::BAD_GATEWAY, "text/plain", "bad gateway").await;
        let client = MockClient::new(base_url).expect("client construction should succeed");

        let error = client
            .request_reply("s_test", "hello")
            .await
            .expect_err("error response should fail");

        assert!(matches!(
            error,
            MockClientError::HttpStatus { status, message }
                if status == StatusCode::BAD_GATEWAY && message == "Bad Gateway"
        ));

        let _ = shutdown_tx.shutdown.send(());
        shutdown_tx
            .handle
            .await
            .expect("test server task should join");
    }

    async fn spawn_error_server(
        status: StatusCode,
        content_type: &'static str,
        body: &'static str,
    ) -> (String, TestServer) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test server should bind");
        let address = listener
            .local_addr()
            .expect("test server address should be readable");
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let router = Router::new().route(
            "/v1/reply",
            post(move || async move { (status, [(CONTENT_TYPE, content_type)], body) }),
        );

        let handle = tokio::spawn(async move {
            let shutdown = async move {
                let _ = shutdown_rx.await;
            };
            axum::serve(listener, router)
                .with_graceful_shutdown(shutdown)
                .await
                .expect("test server should stop cleanly");
        });

        (
            format!("http://{address}"),
            TestServer {
                shutdown: shutdown_tx,
                handle,
            },
        )
    }

    async fn spawn_delayed_server(delay: Duration) -> (String, TestServer) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test server should bind");
        let address = listener
            .local_addr()
            .expect("test server address should be readable");
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let router = Router::new().route(
            "/v1/reply",
            post(move || async move {
                tokio::time::sleep(delay).await;
                (
                    StatusCode::OK,
                    [(CONTENT_TYPE, "application/json")],
                    r#"{"text":"late"}"#,
                )
            }),
        );

        let handle = tokio::spawn(async move {
            let shutdown = async move {
                let _ = shutdown_rx.await;
            };
            axum::serve(listener, router)
                .with_graceful_shutdown(shutdown)
                .await
                .expect("test server should stop cleanly");
        });

        (
            format!("http://{address}"),
            TestServer {
                shutdown: shutdown_tx,
                handle,
            },
        )
    }

    struct TestServer {
        shutdown: oneshot::Sender<()>,
        handle: JoinHandle<()>,
    }
}
