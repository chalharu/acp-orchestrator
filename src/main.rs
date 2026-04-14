use std::{
    env,
    ffi::OsString,
    path::Path,
    process::{ExitStatus, Stdio},
    time::Duration,
};

use acp_app_support::init_tracing;
use acp_mock::{MANUAL_CANCEL_TRIGGER, MANUAL_PERMISSION_TRIGGER};
use snafu::prelude::*;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::{Child, Command},
};

type Result<T, E = LauncherError> = std::result::Result<T, E>;
const STARTUP_LINE_TIMEOUT: Duration = Duration::from_secs(35);

#[derive(Debug, Snafu)]
enum LauncherError {
    #[snafu(display("reading the current executable path failed"))]
    CurrentExecutable { source: std::io::Error },

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

    #[snafu(display("missing the ACP server address after `--acp-server`"))]
    MissingAcpServer,

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
    endpoint: String,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct LauncherArgs {
    acp_server: Option<OsString>,
    cli_args: Vec<OsString>,
}

async fn run_with_args(args: Vec<OsString>) -> Result<()> {
    init_tracing();

    if let Some((role, role_args)) = internal_role_request(&args)? {
        return run_internal_role(role, role_args).await;
    }

    let current_executable = env::current_exe().context(CurrentExecutableSnafu)?;
    let launcher_args = split_launcher_args(&args)?;
    let (mut mock, acp_server) =
        resolve_acp_server(&current_executable, launcher_args.acp_server).await?;
    let SpawnedService {
        child: mut backend,
        endpoint: backend_url,
    } = spawn_background_role(
        &current_executable,
        "web backend",
        "backend",
        vec![
            "--port".into(),
            "0".into(),
            "--acp-server".into(),
            acp_server,
        ],
        &[],
    )
    .await?;

    if should_print_mock_verification_hints(&launcher_args.cli_args, mock.is_some()) {
        println!(
            "[hint] bundled mock verification: enter `{MANUAL_PERMISSION_TRIGGER}` to trigger a permission request, then answer with `/approve <request-id>` or `/deny <request-id>`."
        );
        println!(
            "[hint] bundled mock verification: enter `{MANUAL_CANCEL_TRIGGER}` to start a delayed mock reply, then run `/cancel` before the assistant reply arrives."
        );
    }

    let cli_status = spawn_foreground_role(
        &current_executable,
        "cli frontend",
        "cli",
        launcher_args.cli_args,
        &[("ACP_SERVER_URL", backend_url.as_str())],
    )
    .await?;

    shutdown_services(&mut backend, &mut mock).await?;
    ensure_success("cli frontend", cli_status)
}

fn should_print_mock_verification_hints(cli_args: &[OsString], bundled_mock: bool) -> bool {
    bundled_mock && cli_args.first().and_then(|arg| arg.to_str()) == Some("chat")
}

fn internal_role_request(args: &[OsString]) -> Result<Option<(OsString, Vec<OsString>)>> {
    if args.get(1).and_then(|arg| arg.to_str()) != Some("__internal-role") {
        return Ok(None);
    }

    let role = args
        .get(2)
        .cloned()
        .ok_or_else(|| MissingInternalRoleSnafu.build())?;
    let role_args = args.iter().skip(3).cloned().collect::<Vec<_>>();
    Ok(Some((role, role_args)))
}

async fn resolve_acp_server(
    current_executable: &Path,
    acp_server: Option<OsString>,
) -> Result<(Option<Child>, OsString)> {
    if let Some(acp_server) = acp_server {
        return Ok((None, acp_server));
    }

    let SpawnedService {
        child,
        endpoint: mock_address,
    } = spawn_background_role(
        current_executable,
        "acp mock",
        "mock",
        vec!["--port".into(), "0".into()],
        &[],
    )
    .await?;
    Ok((Some(child), OsString::from(mock_address)))
}

async fn shutdown_services(backend: &mut Child, mock: &mut Option<Child>) -> Result<()> {
    terminate_child(backend, "web backend").await?;
    if let Some(mock) = mock.as_mut() {
        terminate_child(mock, "acp mock").await?;
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    run_with_args(env::args_os().collect()).await
}

fn split_launcher_args(all_args: &[OsString]) -> Result<LauncherArgs> {
    let mut launcher_args = LauncherArgs::default();
    let mut args = all_args.iter().skip(1).cloned();

    while let Some(arg) = args.next() {
        if arg.as_os_str() == "--acp-server" {
            let value = args.next().ok_or_else(|| MissingAcpServerSnafu.build())?;
            if value.is_empty() {
                return MissingAcpServerSnafu.fail();
            }
            launcher_args.acp_server = Some(value);
        } else if let Some(value) = arg
            .to_str()
            .and_then(|value| value.strip_prefix("--acp-server="))
        {
            if value.is_empty() {
                return MissingAcpServerSnafu.fail();
            }
            launcher_args.acp_server = Some(OsString::from(value));
        } else {
            launcher_args.cli_args.push(arg);
        }
    }

    if launcher_args.cli_args.is_empty() {
        launcher_args.cli_args = vec!["chat".into(), "--new".into()];
    }

    Ok(launcher_args)
}

async fn spawn_background_role(
    current_executable: &Path,
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
    let endpoint = read_startup_url(&mut child, role_label).await?;
    Ok(SpawnedService { child, endpoint })
}

async fn spawn_foreground_role(
    current_executable: &Path,
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
    current_executable: &Path,
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
    let bytes_read = tokio::time::timeout(STARTUP_LINE_TIMEOUT, reader.read_line(&mut line))
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
mod tests;
