use acp_contracts::{AssistantReplyRequest, AssistantReplyResponse, ErrorResponse};
use reqwest::{Client, StatusCode};
use snafu::prelude::*;

type Result<T, E = MockClientError> = std::result::Result<T, E>;

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
        let client = Client::builder().build().context(BuildHttpClientSnafu)?;

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
