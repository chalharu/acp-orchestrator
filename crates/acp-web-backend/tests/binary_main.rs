use std::{process::Stdio, time::Duration};

use acp_app_support::{build_http_client_for_url, read_startup_url, wait_for_tcp_connect};
use acp_contracts::HealthResponse;
use acp_mock::{MockConfig, spawn_with_shutdown_task};
use tokio::{net::TcpListener, process::Command, sync::oneshot, time::timeout};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
const BROKEN_PROXY_URL: &str = "http://127.0.0.1:9";

#[tokio::test]
async fn backend_binary_serves_health_checks_even_with_proxy_env() -> Result<()> {
    let (mock_address, mock_shutdown) = spawn_mock_server().await?;
    let mut command = Command::new(env!("CARGO_BIN_EXE_acp-web-backend"));
    command
        .arg("--port")
        .arg("0")
        .arg("--acp-server")
        .arg(&mock_address)
        .arg("--exit-after-ms")
        .arg("500")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    configure_broken_proxy_env(&mut command);
    let mut child = command.spawn()?;
    let base_url = read_startup_url(&mut child, "web backend listening on ").await?;
    let client = build_http_client_for_url(&base_url, Some(Duration::from_secs(1)))?;
    let health: HealthResponse = client
        .get(format!("{base_url}/healthz"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(health.status, "ok");
    let app_response = client
        .get(format!("{base_url}/app/"))
        .send()
        .await?
        .error_for_status()?;
    let app_body = app_response.text().await?;
    assert!(app_body.contains("ACP Web MVP slice 0"));

    let status = timeout(Duration::from_secs(2), child.wait()).await??;
    assert!(status.success());
    let _ = mock_shutdown.send(());
    Ok(())
}

fn configure_broken_proxy_env(command: &mut Command) {
    command
        .env_remove("NO_PROXY")
        .env_remove("no_proxy")
        .env("HTTP_PROXY", BROKEN_PROXY_URL)
        .env("HTTPS_PROXY", BROKEN_PROXY_URL)
        .env("ALL_PROXY", BROKEN_PROXY_URL)
        .env("http_proxy", BROKEN_PROXY_URL)
        .env("https_proxy", BROKEN_PROXY_URL)
        .env("all_proxy", BROKEN_PROXY_URL);
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
