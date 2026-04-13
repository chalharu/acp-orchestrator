use std::{
    env,
    ffi::OsString,
    path::PathBuf,
    process::{ExitStatus, Stdio},
    time::Duration,
};

use acp_app_support::init_tracing;
use reqwest::Client;
use snafu::prelude::*;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::{Child, Command},
    time::{Instant, sleep},
};

type Result<T, E = LauncherError> = std::result::Result<T, E>;

#[derive(Debug, Snafu)]
enum LauncherError {
    #[snafu(display("reading the current executable path failed"))]
    CurrentExecutable { source: std::io::Error },

    #[snafu(display("building the launcher HTTP client failed"))]
    BuildHttpClient { source: reqwest::Error },

    #[snafu(display("spawning the {role} child process failed"))]
    SpawnChild {
        source: std::io::Error,
        role: &'static str,
    },

    #[snafu(display("checking the {role} child process status failed"))]
    CheckChildStatus {
        source: std::io::Error,
        role: &'static str,
    },

    #[snafu(display("{role} exited before becoming healthy"))]
    ChildExitedEarly {
        role: &'static str,
        status: ExitStatus,
    },

    #[snafu(display("{role} did not become healthy at {url}"))]
    WaitForHealth { role: &'static str, url: String },

    #[snafu(display("waiting for the {role} child process failed"))]
    WaitForChild {
        source: std::io::Error,
        role: &'static str,
    },

    #[snafu(display("capturing the {role} child stdout failed"))]
    MissingChildStdout { role: &'static str },

    #[snafu(display("reading the {role} startup line failed"))]
    ReadStartupLine {
        source: std::io::Error,
        role: &'static str,
    },

    #[snafu(display("timed out waiting for the {role} startup line"))]
    WaitForStartupLine { role: &'static str },

    #[snafu(display("the {role} startup line was invalid: {line}"))]
    InvalidStartupLine { role: &'static str, line: String },

    #[snafu(display("{role} exited with status code {code:?}"))]
    ChildExit {
        role: &'static str,
        code: Option<i32>,
    },

    #[snafu(display("terminating the {role} child process failed"))]
    TerminateChild {
        source: std::io::Error,
        role: &'static str,
    },

    #[snafu(display("missing the internal role name"))]
    MissingInternalRole,

    #[snafu(display("unknown internal role `{role}`"))]
    UnknownInternalRole { role: String },

    #[snafu(display("running the cli child failed: {message}"))]
    RunCli { message: String },

    #[snafu(display("running the mock child failed: {message}"))]
    RunMock { message: String },

    #[snafu(display("running the backend child failed: {message}"))]
    RunBackend { message: String },
}

struct SpawnedService {
    child: Child,
    base_url: String,
}

async fn run_with_args(args: Vec<OsString>) -> Result<()> {
    init_tracing();

    if args.get(1).and_then(|arg| arg.to_str()) == Some("__internal-role") {
        let role = args
            .get(2)
            .cloned()
            .ok_or_else(|| MissingInternalRoleSnafu.build())?;
        let role_args = args.into_iter().skip(3).collect::<Vec<_>>();
        return run_internal_role(role, role_args).await;
    }

    let current_executable = env::current_exe().context(CurrentExecutableSnafu)?;
    let cli_args = forwarded_cli_args(&args);
    let client = Client::builder()
        .timeout(Duration::from_millis(200))
        .build()
        .context(BuildHttpClientSnafu)?;

    let SpawnedService {
        child: mut mock,
        base_url: mock_url,
    } = spawn_background_role(
        &current_executable,
        "acp mock",
        "mock",
        vec!["--port".into(), "0".into()],
        &[],
    )
    .await?;
    wait_for_health(&client, &mut mock, "acp mock", &mock_url).await?;

    let SpawnedService {
        child: mut backend,
        base_url: backend_url,
    } = spawn_background_role(
        &current_executable,
        "web backend",
        "backend",
        vec![
            "--port".into(),
            "0".into(),
            "--mock-url".into(),
            mock_url.clone().into(),
        ],
        &[],
    )
    .await?;
    wait_for_health(&client, &mut backend, "web backend", &backend_url).await?;

    let cli_status = spawn_foreground_role(
        &current_executable,
        "cli frontend",
        "cli",
        cli_args,
        &[("ACP_SERVER_URL", backend_url.as_str())],
    )
    .await?;

    terminate_child(&mut backend, "web backend").await?;
    terminate_child(&mut mock, "acp mock").await?;

    ensure_success("cli frontend", cli_status)
}

#[tokio::main]
async fn main() -> Result<()> {
    run_with_args(env::args_os().collect()).await
}

fn forwarded_cli_args(all_args: &[OsString]) -> Vec<OsString> {
    let args = all_args.iter().skip(1).cloned().collect::<Vec<_>>();
    if args.is_empty() {
        vec!["chat".into(), "--new".into()]
    } else {
        args
    }
}

async fn spawn_background_role(
    current_executable: &PathBuf,
    role_label: &'static str,
    role_name: &'static str,
    role_args: Vec<OsString>,
    envs: &[(&str, &str)],
) -> Result<SpawnedService> {
    let mut command = role_command(current_executable, role_name, role_args, envs);
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::inherit());
    command.kill_on_drop(true);
    let mut child = command
        .spawn()
        .context(SpawnChildSnafu { role: role_label })?;
    let base_url = read_startup_url(&mut child, role_label).await?;
    Ok(SpawnedService { child, base_url })
}

async fn spawn_foreground_role(
    current_executable: &PathBuf,
    role_label: &'static str,
    role_name: &'static str,
    role_args: Vec<OsString>,
    envs: &[(&str, &str)],
) -> Result<ExitStatus> {
    let mut child = role_command(current_executable, role_name, role_args, envs)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .context(SpawnChildSnafu { role: role_label })?;

    child
        .wait()
        .await
        .context(WaitForChildSnafu { role: role_label })
}

fn role_command(
    current_executable: &PathBuf,
    role_name: &'static str,
    role_args: Vec<OsString>,
    envs: &[(&str, &str)],
) -> Command {
    let mut command = Command::new(current_executable);
    command.arg("__internal-role").arg(role_name);

    for arg in role_args {
        command.arg(arg);
    }

    for (key, value) in envs {
        command.env(key, value);
    }

    command
}

async fn read_startup_url(child: &mut Child, role: &'static str) -> Result<String> {
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| MissingChildStdoutSnafu { role }.build())?;
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    let bytes_read = tokio::time::timeout(Duration::from_secs(15), reader.read_line(&mut line))
        .await
        .map_err(|_| LauncherError::WaitForStartupLine { role })?
        .context(ReadStartupLineSnafu { role })?;

    if bytes_read == 0 {
        return InvalidStartupLineSnafu {
            role,
            line: "<empty>".to_string(),
        }
        .fail();
    }

    let prefix = "listening on ";
    let line = line.trim();
    let base_url = line
        .split_once(prefix)
        .map(|(_, value)| value.to_string())
        .ok_or_else(|| {
            InvalidStartupLineSnafu {
                role,
                line: line.to_string(),
            }
            .build()
        })?;

    Ok(base_url)
}

async fn wait_for_health(
    client: &Client,
    child: &mut Child,
    role: &'static str,
    base_url: &str,
) -> Result<()> {
    wait_for_health_with_timeout(client, child, role, base_url, Duration::from_secs(15)).await
}

async fn wait_for_health_with_timeout(
    client: &Client,
    child: &mut Child,
    role: &'static str,
    base_url: &str,
    timeout: Duration,
) -> Result<()> {
    let health_url = format!("{base_url}/healthz");
    let deadline = Instant::now() + timeout;

    loop {
        if let Some(status) = child.try_wait().context(CheckChildStatusSnafu { role })? {
            return ChildExitedEarlySnafu { role, status }.fail();
        }

        if let Ok(response) = client.get(&health_url).send().await
            && response.status().is_success()
        {
            return Ok(());
        }

        if Instant::now() >= deadline {
            return WaitForHealthSnafu {
                role,
                url: health_url,
            }
            .fail();
        }

        sleep(Duration::from_millis(100)).await;
    }
}

async fn terminate_child(child: &mut Child, role: &'static str) -> Result<()> {
    if child
        .try_wait()
        .context(CheckChildStatusSnafu { role })?
        .is_some()
    {
        return Ok(());
    }

    child.kill().await.context(TerminateChildSnafu { role })?;
    let _ = child.wait().await.context(WaitForChildSnafu { role })?;
    Ok(())
}

fn ensure_success(role: &'static str, status: ExitStatus) -> Result<()> {
    if status.success() {
        Ok(())
    } else {
        ChildExitSnafu {
            role,
            code: status.code(),
        }
        .fail()
    }
}

async fn run_internal_role(role: OsString, role_args: Vec<OsString>) -> Result<()> {
    match role.to_string_lossy().as_ref() {
        "cli" => {
            let args = std::iter::once(OsString::from("acp")).chain(role_args);
            acp_cli::run_with_args(args)
                .await
                .map_err(|error| LauncherError::RunCli {
                    message: error.to_string(),
                })
        }
        "mock" => run_mock_role(role_args).await,
        "backend" => run_backend_role(role_args).await,
        value => UnknownInternalRoleSnafu {
            role: value.to_string(),
        }
        .fail(),
    }
}

async fn run_mock_role(role_args: Vec<OsString>) -> Result<()> {
    let args = std::iter::once(OsString::from("acp-mock")).chain(role_args);
    acp_mock::run_with_args(args)
        .await
        .map_err(|error| LauncherError::RunMock {
            message: error.to_string(),
        })
}

async fn run_backend_role(role_args: Vec<OsString>) -> Result<()> {
    let args = std::iter::once(OsString::from("acp-web-backend")).chain(role_args);
    acp_web_backend::run_with_args(args)
        .await
        .map_err(|error| LauncherError::RunBackend {
            message: error.to_string(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forwarded_cli_args_defaults_to_chat_new() {
        let args = vec![OsString::from("acp")];

        assert_eq!(
            forwarded_cli_args(&args),
            vec![OsString::from("chat"), OsString::from("--new")]
        );
    }

    #[test]
    fn forwarded_cli_args_preserves_explicit_arguments() {
        let args = vec![
            OsString::from("acp"),
            OsString::from("session"),
            OsString::from("list"),
        ];

        assert_eq!(
            forwarded_cli_args(&args),
            vec![OsString::from("session"), OsString::from("list")]
        );
    }

    #[tokio::test]
    async fn run_with_args_requires_an_internal_role_name() {
        let error = run_with_args(vec![
            OsString::from("acp"),
            OsString::from("__internal-role"),
        ])
        .await
        .expect_err("missing internal role should fail");

        assert!(matches!(error, LauncherError::MissingInternalRole));
    }

    #[tokio::test]
    async fn read_startup_url_rejects_empty_stdout() {
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(":")
            .stdout(Stdio::piped())
            .spawn()
            .expect("child should spawn");

        let error = read_startup_url(&mut child, "test role")
            .await
            .expect_err("empty stdout should fail");

        assert!(matches!(
            error,
            LauncherError::InvalidStartupLine { line, .. } if line == "<empty>"
        ));
    }

    #[tokio::test]
    async fn read_startup_url_rejects_invalid_stdout() {
        let mut child = Command::new("sh")
            .arg("-c")
            .arg("printf 'ready\\n'")
            .stdout(Stdio::piped())
            .spawn()
            .expect("child should spawn");

        let error = read_startup_url(&mut child, "test role")
            .await
            .expect_err("invalid stdout should fail");

        assert!(matches!(
            error,
            LauncherError::InvalidStartupLine { line, .. } if line == "ready"
        ));
    }

    #[tokio::test]
    async fn wait_for_health_detects_exited_children() {
        let client = Client::builder()
            .timeout(Duration::from_millis(20))
            .build()
            .expect("client should build");
        let mut child = Command::new("sh")
            .arg("-c")
            .arg("exit 0")
            .spawn()
            .expect("child should spawn");

        let error = wait_for_health(&client, &mut child, "test role", "http://127.0.0.1:9")
            .await
            .expect_err("exited child should fail");

        assert!(matches!(error, LauncherError::ChildExitedEarly { .. }));
    }

    #[tokio::test]
    async fn wait_for_health_times_out_when_child_stays_running() {
        let client = Client::builder()
            .timeout(Duration::from_millis(20))
            .build()
            .expect("client should build");
        let mut child = Command::new("sh")
            .arg("-c")
            .arg("sleep 1")
            .spawn()
            .expect("child should spawn");

        let error = wait_for_health_with_timeout(
            &client,
            &mut child,
            "test role",
            "http://127.0.0.1:9",
            Duration::from_millis(50),
        )
        .await
        .expect_err("running child should time out");

        assert!(matches!(error, LauncherError::WaitForHealth { .. }));
        terminate_child(&mut child, "test role")
            .await
            .expect("timeout child should terminate cleanly");
    }

    #[tokio::test]
    async fn terminate_child_returns_for_already_exited_processes() {
        let mut child = Command::new("sh")
            .arg("-c")
            .arg("exit 0")
            .spawn()
            .expect("child should spawn");
        let _ = child.wait().await.expect("child should exit");

        terminate_child(&mut child, "test role")
            .await
            .expect("already exited child should be ignored");
    }

    #[test]
    fn ensure_success_rejects_non_zero_exit_codes() {
        let status = std::process::Command::new("sh")
            .arg("-c")
            .arg("exit 3")
            .status()
            .expect("status should be available");

        let error = ensure_success("cli frontend", status).expect_err("non-zero exits should fail");

        assert!(matches!(
            error,
            LauncherError::ChildExit {
                role: "cli frontend",
                code: Some(3)
            }
        ));
    }

    #[tokio::test]
    async fn run_internal_role_reports_unknown_roles() {
        let error = run_internal_role(OsString::from("unknown"), Vec::new())
            .await
            .expect_err("unknown roles should fail");

        assert!(matches!(
            error,
            LauncherError::UnknownInternalRole { role } if role == "unknown"
        ));
    }

    #[tokio::test]
    async fn run_internal_role_wraps_cli_errors() {
        let error = run_internal_role(
            OsString::from("cli"),
            vec![OsString::from("chat"), OsString::from("--new")],
        )
        .await
        .expect_err("invalid cli invocation should fail");

        assert!(matches!(error, LauncherError::RunCli { .. }));
    }

    #[tokio::test]
    async fn run_mock_role_validates_arguments() {
        let error = run_mock_role(vec![OsString::from("--unexpected")])
            .await
            .expect_err("unexpected args should fail");
        assert!(matches!(error, LauncherError::RunMock { .. }));

        let error = run_mock_role(vec![OsString::from("--port"), OsString::from("nope")])
            .await
            .expect_err("invalid ports should fail");
        assert!(matches!(error, LauncherError::RunMock { .. }));
    }

    #[tokio::test]
    async fn run_mock_role_can_shutdown_cleanly() {
        run_mock_role(vec![
            OsString::from("--port"),
            OsString::from("0"),
            OsString::from("--response-delay-ms"),
            OsString::from("1"),
            OsString::from("--exit-after-ms"),
            OsString::from("50"),
        ])
        .await
        .expect("mock role should stop cleanly");
    }

    #[tokio::test]
    async fn run_mock_role_can_start_without_a_test_shutdown() {
        let handle = tokio::spawn(run_mock_role(vec![
            OsString::from("--port"),
            OsString::from("0"),
            OsString::from("--response-delay-ms"),
            OsString::from("1"),
        ]));

        tokio::time::sleep(Duration::from_millis(50)).await;
        handle.abort();
        let _ = handle.await;
    }

    #[tokio::test]
    async fn run_backend_role_validates_arguments() {
        let error = run_backend_role(vec![OsString::from("--unexpected")])
            .await
            .expect_err("unexpected args should fail");
        assert!(matches!(error, LauncherError::RunBackend { .. }));

        let error = run_backend_role(Vec::new())
            .await
            .expect_err("missing mock url should fail");
        assert!(matches!(error, LauncherError::RunBackend { .. }));

        let error = run_backend_role(vec![
            OsString::from("--session-cap"),
            OsString::from("nope"),
        ])
        .await
        .expect_err("invalid session caps should fail");
        assert!(matches!(error, LauncherError::RunBackend { .. }));
    }

    #[tokio::test]
    async fn run_backend_role_can_shutdown_cleanly() {
        run_backend_role(vec![
            OsString::from("--port"),
            OsString::from("0"),
            OsString::from("--mock-url"),
            OsString::from("http://127.0.0.1:9"),
            OsString::from("--exit-after-ms"),
            OsString::from("50"),
        ])
        .await
        .expect("backend role should stop cleanly");
    }

    #[tokio::test]
    async fn run_backend_role_can_start_without_a_test_shutdown() {
        let handle = tokio::spawn(run_backend_role(vec![
            OsString::from("--port"),
            OsString::from("0"),
            OsString::from("--mock-url"),
            OsString::from("http://127.0.0.1:9"),
        ]));

        tokio::time::sleep(Duration::from_millis(50)).await;
        handle.abort();
        let _ = handle.await;
    }
}
