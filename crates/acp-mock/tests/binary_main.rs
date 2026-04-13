use std::{io, process::Stdio, time::Duration};

use acp_contracts::{AssistantReplyRequest, AssistantReplyResponse, HealthResponse};
use reqwest::Client;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::{Child, Command},
    time::{sleep, timeout},
};

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
    let base_url = read_startup_url(&mut child).await?;
    let client = Client::builder().build()?;

    wait_for_health(&client, &base_url).await?;

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

async fn read_startup_url(child: &mut Child) -> Result<String> {
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("missing child stdout"))?;
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader.read_line(&mut line).await?;
    Ok(line
        .trim()
        .strip_prefix("acp mock listening on ")
        .ok_or_else(|| io::Error::other(format!("unexpected startup line: {}", line.trim())))?
        .to_string())
}

async fn wait_for_health(client: &Client, base_url: &str) -> Result<()> {
    let health_url = format!("{base_url}/healthz");
    for _ in 0..100 {
        if let Ok(response) = client.get(&health_url).send().await
            && response.status().is_success()
        {
            return Ok(());
        }
        sleep(Duration::from_millis(20)).await;
    }

    Err(io::Error::other(format!("health check did not succeed for {health_url}")).into())
}
