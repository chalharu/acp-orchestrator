use std::{io, path::PathBuf, process::Stdio, time::Duration};

use acp_app_support::{unique_temp_json_path, wait_for_tcp_connect};
use acp_mock::{MockConfig, spawn_with_shutdown_task};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::oneshot,
    time::sleep,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
const BROKEN_PROXY_URL: &str = "http://127.0.0.1:9";
const MANAGED_STACK_EXIT_AFTER_MS: &str = "15000";

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
    let state_path = unique_launcher_state_path(label);
    let (child, mut stdin, mut reader) = spawn_launcher(
        &recent_path,
        &state_path,
        use_broken_proxy_env,
        launcher_args,
    )?;
    let mut child = child;

    stdin.write_all(b"hello from launcher\n").await?;
    let (_session_id, _backend_url, mut captured_stdout) =
        read_session_connection(&mut reader).await?;
    captured_stdout.push_str(&read_until_output(&mut reader, "[assistant] mock assistant:").await?);
    captured_stdout.push_str(&quit_launcher(&mut child, &mut stdin, &mut reader).await?);
    assert_launcher_output(&captured_stdout);

    Ok(())
}

#[tokio::test]
async fn launcher_reuses_the_bundled_stack_across_invocations() -> Result<()> {
    let label = "launcher-resume";
    let recent_path = unique_recent_sessions_path(label);
    let state_path = unique_launcher_state_path(label);

    let (session_id, backend_url, _first_output) =
        run_bundled_launcher_chat(&recent_path, &state_path).await?;

    let list_output = run_launcher_command(&recent_path, &state_path, ["session", "list"]).await?;
    assert!(list_output.status.success());
    let list_stdout = String::from_utf8(list_output.stdout)?;
    assert!(list_stdout.contains(&session_id));

    let (resumed_session_id, resumed_backend_url, resumed_output) =
        resume_bundled_launcher_chat(&recent_path, &state_path, &session_id).await?;

    assert_eq!(resumed_session_id, session_id);
    assert_eq!(resumed_backend_url, backend_url);
    assert!(resumed_output.contains("[user] hello from launcher"));
    assert!(resumed_output.contains("[assistant] mock assistant:"));

    sleep(Duration::from_millis(5200)).await;
    Ok(())
}

fn spawn_launcher(
    recent_path: &PathBuf,
    state_path: &PathBuf,
    use_broken_proxy_env: bool,
    launcher_args: &[&str],
) -> Result<(Child, ChildStdin, BufReader<ChildStdout>)> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_acp"));
    command
        .env("ACP_RECENT_SESSIONS_PATH", recent_path)
        .env("ACP_LAUNCHER_STATE_PATH", state_path)
        .env(
            "ACP_LAUNCHER_STACK_EXIT_AFTER_MS",
            MANAGED_STACK_EXIT_AFTER_MS,
        )
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

async fn run_bundled_launcher_chat(
    recent_path: &PathBuf,
    state_path: &PathBuf,
) -> Result<(String, String, String)> {
    let (child, mut stdin, mut reader) = spawn_launcher(recent_path, state_path, false, &[])?;
    let mut child = child;
    stdin.write_all(b"hello from launcher\n").await?;
    let (session_id, backend_url, mut output) = read_session_connection(&mut reader).await?;
    output.push_str(&read_until_output(&mut reader, "[assistant] mock assistant:").await?);
    output.push_str(&quit_launcher(&mut child, &mut stdin, &mut reader).await?);
    Ok((session_id, backend_url, output))
}

async fn resume_bundled_launcher_chat(
    recent_path: &PathBuf,
    state_path: &PathBuf,
    session_id: &str,
) -> Result<(String, String, String)> {
    let (child, mut stdin, mut reader) = spawn_launcher(
        recent_path,
        state_path,
        false,
        &["chat", "--session", session_id],
    )?;
    let mut child = child;
    let (resumed_session_id, resumed_backend_url, mut output) =
        read_session_connection(&mut reader).await?;
    output.push_str(&read_until_output(&mut reader, "[assistant] mock assistant:").await?);
    output.push_str(&quit_launcher(&mut child, &mut stdin, &mut reader).await?);
    Ok((resumed_session_id, resumed_backend_url, output))
}

async fn read_until_output(reader: &mut BufReader<ChildStdout>, needle: &str) -> Result<String> {
    let mut captured = String::new();

    for _ in 0..40 {
        let mut line = String::new();
        match tokio::time::timeout(Duration::from_millis(200), reader.read_line(&mut line)).await {
            Ok(Ok(0)) => break,
            Ok(Ok(_)) => {
                captured.push_str(&line);
                if captured.contains(needle) {
                    return Ok(captured);
                }
            }
            Ok(Err(error)) => return Err(error.into()),
            Err(_) => {}
        }
    }

    Ok(captured)
}

fn assert_launcher_output(output: &str) {
    assert!(output.contains("session: s_"));
    assert!(output.contains("connected to backend: http://127.0.0.1:"));
    assert!(output.contains("[assistant] mock assistant:"));
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

fn unique_launcher_state_path(label: &str) -> PathBuf {
    unique_temp_json_path("acp-launcher-state", label)
}

async fn run_launcher_command<'a, I>(
    recent_path: &PathBuf,
    state_path: &PathBuf,
    args: I,
) -> Result<std::process::Output>
where
    I: IntoIterator<Item = &'a str>,
{
    Ok(Command::new(env!("CARGO_BIN_EXE_acp"))
        .env("ACP_RECENT_SESSIONS_PATH", recent_path)
        .env("ACP_LAUNCHER_STATE_PATH", state_path)
        .env(
            "ACP_LAUNCHER_STACK_EXIT_AFTER_MS",
            MANAGED_STACK_EXIT_AFTER_MS,
        )
        .args(args)
        .output()
        .await?)
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
