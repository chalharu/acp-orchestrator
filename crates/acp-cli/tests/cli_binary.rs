use std::{io, path::PathBuf, process::Stdio, time::Duration};

use acp_app_support::{build_http_client_for_url, wait_for_health, wait_for_tcp_connect};
use acp_contracts::{MessageRole, SessionHistoryResponse};
use acp_mock::{MockConfig, spawn_with_shutdown_task};
use acp_web_backend::{AppState, ServerConfig, serve_with_shutdown as serve_backend_with_shutdown};
use reqwest::Client;
use tokio::{
    io::AsyncWriteExt,
    net::TcpListener,
    process::{Child, ChildStdin, Command},
    sync::oneshot,
    time::sleep,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

fn test_state_dir() -> PathBuf {
    std::env::temp_dir().join(format!(
        "acp-cli-backend-test-{}",
        uuid::Uuid::new_v4().simple()
    ))
}

#[tokio::test]
async fn session_list_reports_an_empty_owner_index() -> Result<()> {
    let stack = TestStack::spawn("empty-list").await?;
    let output = run_command([
        "session",
        "list",
        "--server-url",
        stack.backend_url.as_str(),
    ])
    .await?;

    assert!(output.status.success());
    assert!(String::from_utf8(output.stdout)?.contains("no sessions found for the current owner"));
    Ok(())
}

#[tokio::test]
async fn chat_and_session_commands_roundtrip_against_a_backend() -> Result<()> {
    let stack = TestStack::spawn("roundtrip").await?;
    let (session_id, chat_stdout) = run_new_chat_roundtrip(&stack).await?;

    assert_chat_output(&chat_stdout);
    assert_assistant_history(&stack, &session_id).await?;
    assert_resume_and_list_commands(&stack, &session_id).await?;
    close_session_and_reopen_read_only(&stack, &session_id).await?;
    Ok(())
}

#[tokio::test]
async fn chat_command_reports_usage_and_http_errors() -> Result<()> {
    let mode_error = Command::new(env!("CARGO_BIN_EXE_acp-cli"))
        .args(["chat", "--server-url", "http://127.0.0.1:9"])
        .output()
        .await?;
    assert!(!mode_error.status.success());
    assert!(String::from_utf8(mode_error.stderr)?.contains("ChatModeRequired"));

    let server_error = Command::new(env!("CARGO_BIN_EXE_acp-cli"))
        .args(["chat", "--new"])
        .output()
        .await?;
    assert!(!server_error.status.success());
    assert!(String::from_utf8(server_error.stderr)?.contains("MissingServerUrl"));

    let stack = TestStack::spawn("errors-stack").await?;

    let missing_session = run_command([
        "chat",
        "--session",
        "s_missing",
        "--server-url",
        stack.backend_url.as_str(),
    ])
    .await?;
    assert!(!missing_session.status.success());
    assert!(String::from_utf8(missing_session.stderr)?.contains("HttpStatus"));

    Ok(())
}

#[tokio::test]
async fn chat_exits_cleanly_on_immediate_eof() -> Result<()> {
    let stack = TestStack::spawn("eof").await?;
    let mut child =
        spawn_interactive_command(["chat", "--new", "--server-url", stack.backend_url.as_str()])?;
    drop(take_child_stdin(&mut child, "missing eof chat stdin")?);
    let output = child.wait_with_output().await?;

    assert!(output.status.success());
    Ok(())
}

struct TestStack {
    client: Client,
    backend_url: String,
    backend_shutdown: Option<oneshot::Sender<()>>,
    mock_shutdown: Option<oneshot::Sender<()>>,
}

impl TestStack {
    async fn spawn(label: &str) -> Result<Self> {
        let _ = label;
        let (mock_address, mock_shutdown) = spawn_mock_server().await?;
        let (backend_url, backend_shutdown) = spawn_backend_server(mock_address).await?;
        let client = build_http_client_for_url(&backend_url, None)?;
        wait_for_health(&client, &backend_url, 100, Duration::from_millis(20)).await?;

        Ok(Self {
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

const CHAT_SCRIPT: [(&[u8], u64); 8] = [
    (b"\n/help\nhello from cli binary\n", 600),
    (b"verify permission\n", 300),
    (b"/approve req_1\n", 300),
    (b"verify permission\n", 300),
    (b"/deny req_2\n", 300),
    (b"verify cancel\n", 300),
    (b"/cancel\n", 300),
    (b"/unknown\n/quit\n", 0),
];

async fn run_new_chat_roundtrip(stack: &TestStack) -> Result<(String, String)> {
    let mut chat =
        spawn_interactive_command(["chat", "--new", "--server-url", stack.backend_url.as_str()])?;
    let mut stdin = take_child_stdin(&mut chat, "missing chat stdin")?;
    write_chat_script(&mut stdin).await?;
    drop(stdin);

    let output = chat.wait_with_output().await?;
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    Ok((extract_session_id(&stdout)?, stdout))
}

async fn write_chat_script(stdin: &mut ChildStdin) -> Result<()> {
    for (input, delay_ms) in CHAT_SCRIPT {
        stdin.write_all(input).await?;
        sleep(chat_script_delay(delay_ms)).await;
    }
    Ok(())
}

fn chat_script_delay(delay_ms: u64) -> Duration {
    let scaled_delay_ms = if std::env::var_os("LLVM_PROFILE_FILE").is_some() {
        delay_ms.saturating_mul(5)
    } else {
        delay_ms.saturating_mul(3)
    };
    Duration::from_millis(scaled_delay_ms)
}

fn extract_session_id(output: &str) -> Result<String> {
    output
        .lines()
        .find_map(|line| line.strip_prefix("session: ").map(str::to_string))
        .ok_or_else(|| io::Error::other("missing session id in chat output").into())
}

fn assert_chat_output(output: &str) {
    assert!(output.contains("session: s_"));
    assert!(output.contains("connected to backend:"));
    assert!(output.contains("[status] new session ready"));
    assert!(output.contains("[status] available slash commands:"));
    assert!(output.contains("/help"));
    assert!(output.contains("Show available slash commands"));
    assert!(output.contains("/cancel"));
    assert!(output.contains("Cancel the running turn"));
    assert!(output.contains("/approve <request-id>"));
    assert!(output.contains("Approve a pending permission request"));
    assert!(output.contains("/deny <request-id>"));
    assert!(output.contains("Deny a pending permission request"));
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
    let mut resumed = spawn_interactive_command([
        "chat",
        "--session",
        session_id,
        "--server-url",
        stack.backend_url.as_str(),
    ])?;
    let mut stdin = take_child_stdin(&mut resumed, "missing resumed chat stdin")?;
    stdin.write_all(b"/quit\n").await?;
    drop(stdin);

    let resumed_output = resumed.wait_with_output().await?;
    assert!(resumed_output.status.success());
    let resumed_stdout = String::from_utf8(resumed_output.stdout)?;
    assert!(resumed_stdout.contains(session_id));
    assert!(resumed_stdout.contains("[status] resumed existing session"));
    assert!(resumed_stdout.contains("[user] hello from cli binary"));
    assert!(resumed_stdout.contains("[assistant] mock assistant:"));

    let list_output = run_command([
        "session",
        "list",
        "--server-url",
        stack.backend_url.as_str(),
    ])
    .await?;
    assert!(list_output.status.success());
    let list_stdout = String::from_utf8(list_output.stdout)?;
    assert!(list_stdout.contains(session_id));
    assert!(list_stdout.contains("active"));
    Ok(())
}

async fn close_session_and_reopen_read_only(stack: &TestStack, session_id: &str) -> Result<()> {
    let close_output = run_command([
        "session",
        "close",
        session_id,
        "--server-url",
        stack.backend_url.as_str(),
    ])
    .await?;
    assert!(close_output.status.success());
    assert!(String::from_utf8(close_output.stdout)?.contains("closed"));

    let list_output = run_command([
        "session",
        "list",
        "--server-url",
        stack.backend_url.as_str(),
    ])
    .await?;
    assert!(list_output.status.success());
    let list_stdout = String::from_utf8(list_output.stdout)?;
    assert!(list_stdout.contains(session_id));
    assert!(list_stdout.contains("closed"));

    let read_only_output = run_command([
        "chat",
        "--session",
        session_id,
        "--server-url",
        stack.backend_url.as_str(),
    ])
    .await?;
    assert!(read_only_output.status.success());
    let read_only_stdout = String::from_utf8(read_only_output.stdout)?;
    assert!(read_only_stdout.contains("[status] opened closed session as read-only transcript"));
    assert!(read_only_stdout.contains("[user] hello from cli binary"));
    assert!(read_only_stdout.contains("[assistant] mock assistant:"));
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

fn spawn_interactive_command<'a, I>(args: I) -> Result<Child>
where
    I: IntoIterator<Item = &'a str>,
{
    Ok(Command::new(env!("CARGO_BIN_EXE_acp-cli"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?)
}

async fn run_command<'a, I>(args: I) -> Result<std::process::Output>
where
    I: IntoIterator<Item = &'a str>,
{
    Ok(Command::new(env!("CARGO_BIN_EXE_acp-cli"))
        .args(args)
        .output()
        .await?)
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
        startup_hints: false,
        state_dir: test_state_dir(),
        frontend_dist: None,
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

    Ok((format!("https://{address}"), shutdown_tx))
}
