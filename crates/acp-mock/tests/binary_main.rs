use std::{process::Stdio, time::Duration};

use acp_app_support::{read_startup_url, wait_for_health};
use acp_contracts::{AssistantReplyRequest, AssistantReplyResponse, HealthResponse};
use reqwest::Client;
use tokio::{process::Command, time::timeout};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[tokio::test]
async fn mock_binary_serves_health_and_reply_requests() -> Result<()> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_acp-mock"))
        .arg("--port")
        .arg("0")
        .arg("--response-delay-ms")
        .arg("1")
        .arg("--exit-after-ms")
        .arg("500")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    let base_url = read_startup_url(&mut child, "acp mock listening on ").await?;
    let client = Client::builder().build()?;

    wait_for_health(&client, &base_url, 100, Duration::from_millis(20)).await?;

    let health: HealthResponse = client
        .get(format!("{base_url}/healthz"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(health.status, "ok");

    let reply: AssistantReplyResponse = client
        .post(format!("{base_url}/v1/reply"))
        .json(&AssistantReplyRequest {
            session_id: "s_test".to_string(),
            prompt: "hello from binary test".to_string(),
        })
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert!(reply.text.starts_with("mock assistant:"));

    let status = timeout(Duration::from_secs(2), child.wait()).await??;
    assert!(status.success());
    Ok(())
}
