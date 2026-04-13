use std::{
    fs, io,
    path::PathBuf,
    process::Stdio,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use acp_contracts::{MessageRole, SessionHistoryResponse};
use acp_mock::{MockConfig, serve_with_shutdown as serve_mock_with_shutdown};
use acp_web_backend::{AppState, ServerConfig, serve_with_shutdown as serve_backend_with_shutdown};
use reqwest::Client;
use serde_json::Value;
use tokio::{io::AsyncWriteExt, net::TcpListener, process::Command, sync::oneshot, time::sleep};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[tokio::test]
async fn session_list_reports_an_empty_cache() -> Result<()> {
    let recent_path = unique_recent_sessions_path("empty");
    let output = Command::new(env!("CARGO_BIN_EXE_acp-cli"))
        .args(["session", "list"])
        .env("ACP_RECENT_SESSIONS_PATH", &recent_path)
        .output()
        .await?;

    assert!(output.status.success());
    assert!(String::from_utf8(output.stdout)?.contains("no recent sessions recorded"));
    Ok(())
}

#[tokio::test]
async fn chat_and_session_commands_roundtrip_against_a_backend() -> Result<()> {
    let recent_path = unique_recent_sessions_path("roundtrip");
    let client = Client::builder().build()?;
    let (mock_url, mock_shutdown) = spawn_mock_server().await?;
    let (backend_url, backend_shutdown) = spawn_backend_server(mock_url).await?;
    wait_for_health(&client, &backend_url).await?;

    let mut chat = Command::new(env!("CARGO_BIN_EXE_acp-cli"))
        .args(["chat", "--new", "--server-url", &backend_url])
        .env("ACP_RECENT_SESSIONS_PATH", &recent_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdin = chat
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("missing chat stdin"))?;
    stdin.write_all(b"\n/help\nhello from cli binary\n").await?;
    sleep(Duration::from_millis(600)).await;
    stdin
        .write_all(b"/cancel\n/approve req-1\n/deny req-1\n/unknown\n/quit\n")
        .await?;
    drop(stdin);

    let chat_output = chat.wait_with_output().await?;
    assert!(chat_output.status.success());
    let chat_stdout = String::from_utf8(chat_output.stdout)?;
    assert!(chat_stdout.contains("session: s_"));
    assert!(chat_stdout.contains("connected to backend:"));
    assert!(chat_stdout.contains("/help"));
    assert!(chat_stdout.contains("[status] `/cancel` is planned."));
    assert!(chat_stdout.contains("[status] `/approve` is planned."));
    assert!(chat_stdout.contains("[status] `/deny` is planned."));
    assert!(chat_stdout.contains("[status] unknown command. Use `/help`."));

    let session_id = read_recent_session_id(&recent_path)?;
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

    let mut resumed = Command::new(env!("CARGO_BIN_EXE_acp-cli"))
        .args([
            "chat",
            "--session",
            &session_id,
            "--server-url",
            &backend_url,
        ])
        .env("ACP_RECENT_SESSIONS_PATH", &recent_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut resumed_stdin = resumed
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("missing resumed chat stdin"))?;
    resumed_stdin.write_all(b"/quit\n").await?;
    drop(resumed_stdin);
    let resumed_output = resumed.wait_with_output().await?;
    assert!(resumed_output.status.success());
    assert!(String::from_utf8(resumed_output.stdout)?.contains(&session_id));

    let list_output = Command::new(env!("CARGO_BIN_EXE_acp-cli"))
        .args(["session", "list"])
        .env("ACP_RECENT_SESSIONS_PATH", &recent_path)
        .output()
        .await?;
    assert!(list_output.status.success());
    assert!(String::from_utf8(list_output.stdout)?.contains(&session_id));

    let close_output = Command::new(env!("CARGO_BIN_EXE_acp-cli"))
        .args([
            "session",
            "close",
            &session_id,
            "--server-url",
            &backend_url,
        ])
        .env("ACP_RECENT_SESSIONS_PATH", &recent_path)
        .output()
        .await?;
    assert!(close_output.status.success());
    assert!(String::from_utf8(close_output.stdout)?.contains("closed"));
    assert!(!fs::read_to_string(&recent_path)?.contains(&session_id));

    let _ = backend_shutdown.send(());
    let _ = mock_shutdown.send(());
    Ok(())
}

#[tokio::test]
async fn chat_command_reports_usage_and_http_errors() -> Result<()> {
    let recent_path = unique_recent_sessions_path("errors");

    let mode_error = Command::new(env!("CARGO_BIN_EXE_acp-cli"))
        .args(["chat", "--server-url", "http://127.0.0.1:9"])
        .env("ACP_RECENT_SESSIONS_PATH", &recent_path)
        .output()
        .await?;
    assert!(!mode_error.status.success());
    assert!(String::from_utf8(mode_error.stderr)?.contains("ChatModeRequired"));

    let server_error = Command::new(env!("CARGO_BIN_EXE_acp-cli"))
        .args(["chat", "--new"])
        .env("ACP_RECENT_SESSIONS_PATH", &recent_path)
        .output()
        .await?;
    assert!(!server_error.status.success());
    assert!(String::from_utf8(server_error.stderr)?.contains("MissingServerUrl"));

    let client = Client::builder().build()?;
    let (mock_url, mock_shutdown) = spawn_mock_server().await?;
    let (backend_url, backend_shutdown) = spawn_backend_server(mock_url).await?;
    wait_for_health(&client, &backend_url).await?;

    let missing_session = Command::new(env!("CARGO_BIN_EXE_acp-cli"))
        .args([
            "chat",
            "--session",
            "s_missing",
            "--server-url",
            &backend_url,
        ])
        .env("ACP_RECENT_SESSIONS_PATH", &recent_path)
        .output()
        .await?;
    assert!(!missing_session.status.success());
    assert!(String::from_utf8(missing_session.stderr)?.contains("HttpStatus"));

    let _ = backend_shutdown.send(());
    let _ = mock_shutdown.send(());
    Ok(())
}

#[tokio::test]
async fn chat_exits_cleanly_on_immediate_eof() -> Result<()> {
    let recent_path = unique_recent_sessions_path("eof");
    let client = Client::builder().build()?;
    let (mock_url, mock_shutdown) = spawn_mock_server().await?;
    let (backend_url, backend_shutdown) = spawn_backend_server(mock_url).await?;
    wait_for_health(&client, &backend_url).await?;

    let mut child = Command::new(env!("CARGO_BIN_EXE_acp-cli"))
        .args(["chat", "--new", "--server-url", &backend_url])
        .env("ACP_RECENT_SESSIONS_PATH", &recent_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("missing eof chat stdin"))?;
    let output = child.wait_with_output().await?;

    assert!(output.status.success());

    let _ = backend_shutdown.send(());
    let _ = mock_shutdown.send(());
    Ok(())
}

fn unique_recent_sessions_path(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after the epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("acp-cli-{label}-{nanos}.json"))
}

fn read_recent_session_id(path: &PathBuf) -> Result<String> {
    let entries: Value = serde_json::from_str(&fs::read_to_string(path)?)?;
    Ok(entries[0]["session_id"]
        .as_str()
        .ok_or_else(|| io::Error::other("missing session id in recent session cache"))?
        .to_string())
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

async fn spawn_backend_server(mock_url: String) -> Result<(String, oneshot::Sender<()>)> {
    let state = AppState::new(ServerConfig {
        session_cap: 8,
        mock_url,
    })?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    tokio::spawn(async move {
        let shutdown = async move {
            let _ = shutdown_rx.await;
        };
        serve_backend_with_shutdown(listener, state, shutdown)
            .await
            .expect("backend server should stop cleanly");
    });

    Ok((format!("http://{address}"), shutdown_tx))
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
