use std::{process::Stdio, time::Duration};

use acp_app_support::{read_startup_url, wait_for_tcp_connect};
use acp_contracts::HealthResponse;
use acp_mock::{MockConfig, spawn_with_shutdown_task};
use reqwest::Client;
use tokio::{net::TcpListener, process::Command, sync::oneshot, time::timeout};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[tokio::test]
async fn backend_binary_serves_health_checks() -> Result<()> {
    let client = Client::builder().build()?;
    let (mock_address, mock_shutdown) = spawn_mock_server().await?;
    let mut child = Command::new(env!("CARGO_BIN_EXE_acp-web-backend"))
        .arg("--port")
        .arg("0")
        .arg("--mock-address")
        .arg(&mock_address)
        .arg("--exit-after-ms")
        .arg("500")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    let base_url = read_startup_url(&mut child, "web backend listening on ").await?;
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

    spawn_with_shutdown_task(listener, MockConfig::default(), async move {
        let _ = shutdown_rx.await;
    });

    wait_for_tcp_connect(&address.to_string(), 20, Duration::from_millis(10)).await?;

    Ok((address.to_string(), shutdown_tx))
}
