use std::{io, path::PathBuf, process::Stdio, time::Duration};

use acp_app_support::unique_temp_json_path;
use acp_contracts::{MessageRole, SessionHistoryResponse};
use reqwest::Client;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::Command,
    time::sleep,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[tokio::test]
async fn launcher_starts_the_full_stack_and_proxies_cli_io() -> Result<()> {
    let recent_path = unique_recent_sessions_path("launcher");
    let client = Client::builder().build()?;
    let mut child = Command::new(env!("CARGO_BIN_EXE_acp"))
        .env("ACP_RECENT_SESSIONS_PATH", &recent_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("missing launcher stdin"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("missing launcher stdout"))?;
    let mut reader = BufReader::new(stdout);

    stdin.write_all(b"hello from launcher\n").await?;
    let (session_id, backend_url, mut captured_stdout) =
        read_session_connection(&mut reader).await?;
    sleep(Duration::from_millis(600)).await;
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
    stdin.write_all(b"/quit\n").await?;
    drop(stdin);

    let mut tail = String::new();
    reader.read_to_string(&mut tail).await?;
    captured_stdout.push_str(&tail);

    let status = child.wait().await?;
    assert!(status.success());
    assert!(captured_stdout.contains("session: s_"));
    assert!(captured_stdout.contains("connected to backend: http://127.0.0.1:"));

    Ok(())
}

async fn read_session_connection(
    reader: &mut BufReader<tokio::process::ChildStdout>,
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
