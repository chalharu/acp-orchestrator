use std::{io, process::Stdio, time::Duration};

use acp_contracts::HealthResponse;
use acp_mock::{MockConfig, serve_with_shutdown as serve_mock_with_shutdown};
use reqwest::Client;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    net::TcpListener,
    process::{Child, Command},
    sync::oneshot,
    time::{sleep, timeout},
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[tokio::test]
async fn backend_binary_serves_health_checks() -> Result<()> {
    let client = Client::builder().build()?;
    let (mock_url, mock_shutdown) = spawn_mock_server().await?;
    let mut child = Command::new(env!("CARGO_BIN_EXE_acp-web-backend"))
        .arg("--port")
        .arg("0")
        .arg("--mock-url")
        .arg(&mock_url)
        .arg("--exit-after-ms")
        .arg("500")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    let base_url = read_startup_url(&mut child).await?;

    wait_for_health(&client, &base_url).await?;

    let health: HealthResponse = client
        .get(format!("{base_url}/healthz"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(health.status, "ok");

    let status = timeout(Duration::from_secs(2), child.wait()).await??;
    assert!(status.success());
    let _ = mock_shutdown.send(());
    Ok(())
}

async fn spawn_mock_server() -> Result<(String, oneshot::Sender<()>)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    tokio::spawn(async move {
        let shutdown = async move {
            let _ = shutdown_rx.await;
        };
        serve_mock_with_shutdown(listener, MockConfig::default(), shutdown)
            .await
            .expect("mock server should stop cleanly");
    });

    Ok((format!("http://{address}"), shutdown_tx))
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
        .strip_prefix("web backend listening on ")
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
