use std::{process::Stdio, time::Duration};

use acp_mock::{MockConfig, spawn_with_shutdown_task};
use acp_web_backend::contract_health::HealthResponse;
use acp_web_backend::support::http::{build_http_client_for_url, wait_for_tcp_connect};
use acp_web_backend::support::runtime::read_startup_url;
use tokio::{net::TcpListener, process::Command, sync::oneshot, time::timeout};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
const BROKEN_PROXY_URL: &str = "http://127.0.0.1:9";

#[tokio::test]
async fn backend_binary_serves_health_checks_even_with_proxy_env() -> Result<()> {
    let (mock_address, mock_shutdown) = spawn_mock_server().await?;
    let mut child = spawn_backend_binary(&mock_address)?;
    let base_url = read_startup_url(&mut child, "web backend listening on ").await?;
    assert_backend_endpoints(&base_url).await?;

    let status = timeout(Duration::from_secs(2), child.wait()).await??;
    assert!(status.success());
    let _ = mock_shutdown.send(());
    Ok(())
}

fn spawn_backend_binary(mock_address: &str) -> Result<tokio::process::Child> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_acp-web-backend"));
    command
        .arg("--port")
        .arg("0")
        .arg("--acp-server")
        .arg(mock_address)
        .arg("--exit-after-ms")
        .arg("500")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    configure_broken_proxy_env(&mut command);
    command.spawn().map_err(Into::into)
}

async fn assert_backend_endpoints(base_url: &str) -> Result<()> {
    let client = build_http_client_for_url(base_url, Some(Duration::from_secs(1)))?;
    let health: HealthResponse = client
        .get(format!("{base_url}/healthz"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(health.status, "ok");

    let app_body = client
        .get(format!("{base_url}/app/"))
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    // The shell must contain the CSRF bootstrap meta and the Leptos mount point.
    assert!(app_body.contains("name=\"acp-csrf-token\""));
    assert!(app_body.contains("wasm-init.js"));
    assert!(app_body.contains("id=\"app-root\""));
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
