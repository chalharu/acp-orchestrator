use std::{process::Stdio, time::Duration};

use acp_app_support::{read_startup_url, wait_for_health};
use acp_contracts::HealthResponse;
use acp_mock::{MockConfig, serve_with_shutdown as serve_mock_with_shutdown};
use reqwest::Client;
use tokio::{net::TcpListener, process::Command, sync::oneshot, time::timeout};

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
    let base_url = read_startup_url(&mut child, "web backend listening on ").await?;

    wait_for_health(&client, &base_url, 100, Duration::from_millis(20)).await?;

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
