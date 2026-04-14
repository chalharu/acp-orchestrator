use std::{
    fs, io,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use acp_app_support::{unique_temp_json_path, wait_for_health, wait_for_tcp_connect};
use acp_contracts::{MessageRole, SessionHistoryResponse};
use acp_mock::{MockConfig, spawn_with_shutdown_task};
use acp_web_backend::{AppState, ServerConfig, serve_with_shutdown as serve_backend_with_shutdown};
use reqwest::Client;
use serde_json::Value;
use tokio::{
    io::AsyncWriteExt,
    net::TcpListener,
    process::{Child, ChildStdin, Command},
    sync::oneshot,
    time::sleep,
};

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
    let stack = TestStack::spawn("roundtrip").await?;
    let chat_stdout = run_new_chat_roundtrip(&stack).await?;
    let session_id = read_recent_session_id(&stack.recent_path)?;

    assert_chat_output(&chat_stdout);
    assert_assistant_history(&stack, &session_id).await?;
    assert_resume_and_list_commands(&stack, &session_id).await?;
    close_session_and_clear_cache(&stack, &session_id).await?;
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

    let stack = TestStack::spawn("errors-stack").await?;

    let missing_session = run_command(
        [
            "chat",
            "--session",
            "s_missing",
            "--server-url",
            stack.backend_url.as_str(),
        ],
        &recent_path,
    )
    .await?;
    assert!(!missing_session.status.success());
    assert!(String::from_utf8(missing_session.stderr)?.contains("HttpStatus"));

    Ok(())
}

#[tokio::test]
async fn chat_exits_cleanly_on_immediate_eof() -> Result<()> {
    let stack = TestStack::spawn("eof").await?;
    let mut child = spawn_interactive_command(
        ["chat", "--new", "--server-url", stack.backend_url.as_str()],
        &stack.recent_path,
    )?;
    drop(take_child_stdin(&mut child, "missing eof chat stdin")?);
    let output = child.wait_with_output().await?;

    assert!(output.status.success());
    Ok(())
}

struct TestStack {
    recent_path: PathBuf,
    client: Client,
    backend_url: String,
    backend_shutdown: Option<oneshot::Sender<()>>,
    mock_shutdown: Option<oneshot::Sender<()>>,
}

impl TestStack {
    async fn spawn(label: &str) -> Result<Self> {
        let recent_path = unique_recent_sessions_path(label);
        let client = Client::builder().build()?;
        let (mock_address, mock_shutdown) = spawn_mock_server().await?;
        let (backend_url, backend_shutdown) = spawn_backend_server(mock_address).await?;
        wait_for_health(&client, &backend_url, 100, Duration::from_millis(20)).await?;

        Ok(Self {
            recent_path,
            client,
            backend_url,
            backend_shutdown: Some(backend_shutdown),
            mock_shutdown: Some(mock_shutdown),
        })
    }
}

impl Drop for TestStack {
    fn drop(&mut self) {
        if let Some(shutdown) = self.backend_shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(shutdown) = self.mock_shutdown.take() {
            let _ = shutdown.send(());
        }
    }
}

fn unique_recent_sessions_path(label: &str) -> PathBuf {
    unique_temp_json_path("acp-cli", label)
}

fn read_recent_session_id(path: &Path) -> Result<String> {
    let entries: Value = serde_json::from_str(&fs::read_to_string(path)?)?;
    Ok(entries[0]["session_id"]
        .as_str()
        .ok_or_else(|| io::Error::other("missing session id in recent session cache"))?
        .to_string())
}

const CHAT_SCRIPT: [(&[u8], u64); 8] = [
    (b"\n/help\nhello from cli binary\n", 600),
    (b"verify permission\n", 300),
    (b"/approve req_1\n", 300),
    (b"verify permission again\n", 300),
    (b"/deny req_2\n", 300),
    (b"verify cancel\n", 300),
    (b"/cancel\n", 300),
    (b"/unknown\n/quit\n", 0),
];

async fn run_new_chat_roundtrip(stack: &TestStack) -> Result<String> {
    let mut chat = spawn_interactive_command(
        ["chat", "--new", "--server-url", stack.backend_url.as_str()],
        &stack.recent_path,
    )?;
    let mut stdin = take_child_stdin(&mut chat, "missing chat stdin")?;
    write_chat_script(&mut stdin).await?;
    drop(stdin);

    let output = chat.wait_with_output().await?;
    assert!(output.status.success());
    Ok(String::from_utf8(output.stdout)?)
}

async fn write_chat_script(stdin: &mut ChildStdin) -> Result<()> {
    for (input, delay_ms) in CHAT_SCRIPT {
        stdin.write_all(input).await?;
        sleep(Duration::from_millis(delay_ms)).await;
    }
    Ok(())
}

fn assert_chat_output(output: &str) {
    assert!(output.contains("session: s_"));
    assert!(output.contains("connected to backend:"));
    assert!(output.contains("/help"));
    assert!(output.contains("/cancel"));
    assert!(output.contains("/approve <request-id>"));
    assert!(output.contains("/deny <request-id>"));
    assert!(output.contains("[permission req_1] read_text_file README.md"));
    assert!(output.contains("[status] permission req_1 approved"));
    assert!(output.contains("[permission req_2] read_text_file README.md"));
    assert!(output.contains("[status] permission req_2 denied"));
    assert!(output.contains("[user] verify cancel"));
    assert!(output.contains("[status] cancel requested for the running turn"));
    assert!(output.contains("[status] turn cancelled"));
    assert!(output.contains("[status] unknown command. Use `/help`."));
}

async fn assert_assistant_history(stack: &TestStack, session_id: &str) -> Result<()> {
    let history = fetch_history(&stack.client, &stack.backend_url, session_id).await?;
    assert!(
        history
            .messages
            .iter()
            .any(|message| matches!(message.role, MessageRole::Assistant)
                && message.text.starts_with("mock assistant:"))
    );
    Ok(())
}

async fn assert_resume_and_list_commands(stack: &TestStack, session_id: &str) -> Result<()> {
    let mut resumed = spawn_interactive_command(
        [
            "chat",
            "--session",
            session_id,
            "--server-url",
            stack.backend_url.as_str(),
        ],
        &stack.recent_path,
    )?;
    let mut stdin = take_child_stdin(&mut resumed, "missing resumed chat stdin")?;
    stdin.write_all(b"/quit\n").await?;
    drop(stdin);

    let resumed_output = resumed.wait_with_output().await?;
    assert!(resumed_output.status.success());
    assert!(String::from_utf8(resumed_output.stdout)?.contains(session_id));

    let list_output = run_command(["session", "list"], &stack.recent_path).await?;
    assert!(list_output.status.success());
    assert!(String::from_utf8(list_output.stdout)?.contains(session_id));
    Ok(())
}

async fn close_session_and_clear_cache(stack: &TestStack, session_id: &str) -> Result<()> {
    let close_output = run_command(
        [
            "session",
            "close",
            session_id,
            "--server-url",
            stack.backend_url.as_str(),
        ],
        &stack.recent_path,
    )
    .await?;
    assert!(close_output.status.success());
    assert!(String::from_utf8(close_output.stdout)?.contains("closed"));
    assert!(!fs::read_to_string(&stack.recent_path)?.contains(session_id));
    Ok(())
}

async fn fetch_history(
    client: &Client,
    backend_url: &str,
    session_id: &str,
) -> Result<SessionHistoryResponse> {
    client
        .get(format!(
            "{backend_url}/api/v1/sessions/{session_id}/history"
        ))
        .bearer_auth("developer")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await
        .map_err(Into::into)
}

fn cli_command(recent_path: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_acp-cli"));
    command.env("ACP_RECENT_SESSIONS_PATH", recent_path);
    command
}

fn spawn_interactive_command<'a, I>(args: I, recent_path: &Path) -> Result<Child>
where
    I: IntoIterator<Item = &'a str>,
{
    Ok(cli_command(recent_path)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?)
}

async fn run_command<'a, I>(args: I, recent_path: &Path) -> Result<std::process::Output>
where
    I: IntoIterator<Item = &'a str>,
{
    Ok(cli_command(recent_path).args(args).output().await?)
}

fn take_child_stdin(child: &mut Child, message: &str) -> Result<ChildStdin> {
    child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other(message).into())
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

async fn spawn_backend_server(mock_address: String) -> Result<(String, oneshot::Sender<()>)> {
    let state = AppState::new(ServerConfig {
        session_cap: 8,
        acp_server: mock_address,
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
