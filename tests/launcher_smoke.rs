use std::{io, path::PathBuf, process::Stdio, time::Duration};

use acp_app_support::{unique_temp_json_path, wait_for_tcp_connect};
use acp_contracts::{MessageRole, SessionHistoryResponse};
use acp_mock::{MockConfig, spawn_with_shutdown_task};
use reqwest::Client;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::oneshot,
    time::sleep,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
const BROKEN_PROXY_URL: &str = "http://127.0.0.1:9";

#[tokio::test]
async fn launcher_starts_the_full_stack_and_proxies_cli_io() -> Result<()> {
    assert_launcher_roundtrip("launcher", false).await
}

#[tokio::test]
async fn launcher_starts_the_full_stack_and_proxies_cli_io_with_proxy_env() -> Result<()> {
    assert_launcher_roundtrip("launcher-proxy", true).await
}

#[tokio::test]
async fn launcher_connects_to_an_existing_acp_server() -> Result<()> {
    let (acp_server, mock_shutdown) = spawn_mock_server().await?;
    let result = assert_launcher_roundtrip_with_args(
        "launcher-existing-acp",
        false,
        &["--acp-server", acp_server.as_str()],
    )
    .await;
    let _ = mock_shutdown.send(());
    result
}

#[tokio::test]
async fn launcher_connects_to_an_existing_acp_server_with_equals_syntax() -> Result<()> {
    let (acp_server, mock_shutdown) = spawn_mock_server().await?;
    let arg = format!("--acp-server={acp_server}");
    let result =
        assert_launcher_roundtrip_with_args("launcher-existing-acp-equals", false, &[arg.as_str()])
            .await;
    let _ = mock_shutdown.send(());
    result
}

async fn assert_launcher_roundtrip(label: &str, use_broken_proxy_env: bool) -> Result<()> {
    assert_launcher_roundtrip_with_args(label, use_broken_proxy_env, &[]).await
}

async fn assert_launcher_roundtrip_with_args(
    label: &str,
    use_broken_proxy_env: bool,
    launcher_args: &[&str],
) -> Result<()> {
    let recent_path = unique_recent_sessions_path(label);
    let client = Client::builder().build()?;
    let (child, mut stdin, mut reader) =
        spawn_launcher(&recent_path, use_broken_proxy_env, launcher_args)?;
    let mut child = child;

    stdin.write_all(b"hello from launcher\n").await?;
    let (session_id, backend_url, mut captured_stdout) =
        read_session_connection(&mut reader).await?;
    sleep(Duration::from_millis(600)).await;
    assert_assistant_history(&client, &backend_url, &session_id).await?;
    captured_stdout.push_str(&quit_launcher(&mut child, &mut stdin, &mut reader).await?);
    assert_launcher_output(&captured_stdout);

    Ok(())
}

fn spawn_launcher(
    recent_path: &PathBuf,
    use_broken_proxy_env: bool,
    launcher_args: &[&str],
) -> Result<(Child, ChildStdin, BufReader<ChildStdout>)> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_acp"));
    command
        .env("ACP_RECENT_SESSIONS_PATH", recent_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.args(launcher_args);
    if use_broken_proxy_env {
        configure_broken_proxy_env(&mut command);
    }
    let mut child = command.spawn()?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("missing launcher stdin"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("missing launcher stdout"))?;

    Ok((child, stdin, BufReader::new(stdout)))
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

async fn assert_assistant_history(
    client: &Client,
    backend_url: &str,
    session_id: &str,
) -> Result<()> {
    let history: SessionHistoryResponse = client
        .get(format!(
            "{backend_url}/api/v1/sessions/{session_id}/history"
        ))
        .bearer_auth("developer")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    assert!(
        history
            .messages
            .iter()
            .any(|message| matches!(message.role, MessageRole::Assistant)
                && message.text.starts_with("mock assistant:"))
    );
    Ok(())
}

async fn quit_launcher(
    child: &mut Child,
    stdin: &mut ChildStdin,
    reader: &mut BufReader<ChildStdout>,
) -> Result<String> {
    stdin.write_all(b"/quit\n").await?;
    let mut tail = String::new();
    reader.read_to_string(&mut tail).await?;

    let status = child.wait().await?;
    assert!(status.success());
    Ok(tail)
}

fn assert_launcher_output(output: &str) {
    assert!(output.contains("session: s_"));
    assert!(output.contains("connected to backend: http://127.0.0.1:"));
}

async fn read_session_connection(
    reader: &mut BufReader<ChildStdout>,
) -> Result<(String, String, String)> {
    let mut session_id = None;
    let mut backend_url = None;
    let mut captured = String::new();

    for _ in 0..40 {
        let mut line = String::new();
        let bytes = reader.read_line(&mut line).await?;
        if bytes == 0 {
            break;
        }

        if let Some(value) = line.strip_prefix("session: ") {
            session_id = Some(value.trim().to_string());
        }
        if let Some(value) = line.strip_prefix("connected to backend: ") {
            backend_url = Some(value.trim().to_string());
        }
        captured.push_str(&line);

        if let (Some(session_id), Some(backend_url)) = (session_id.clone(), backend_url.clone()) {
            return Ok((session_id, backend_url, captured));
        }
    }

    Err(io::Error::other("launcher did not print session and backend connection lines").into())
}

fn unique_recent_sessions_path(label: &str) -> PathBuf {
    unique_temp_json_path("acp-launcher", label)
}

async fn spawn_mock_server() -> Result<(String, oneshot::Sender<()>)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    spawn_with_shutdown_task(listener, MockConfig::default(), async move {
        let _ = shutdown_rx.await;
    });

    wait_for_tcp_connect(&address.to_string(), 100, Duration::from_millis(20)).await?;

    Ok((address.to_string(), shutdown_tx))
}
