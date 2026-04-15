use std::{
    env,
    ffi::OsString,
    path::{Path, PathBuf},
};

use acp_app_support::init_tracing;
use snafu::prelude::*;

mod launcher_process;
mod launcher_stack;

use launcher_process::{ensure_success, spawn_foreground_role};
use launcher_stack::prepare_launcher_stack;

#[cfg(test)]
pub(crate) use launcher_process::{read_startup_url, terminate_child};
#[cfg(test)]
pub(crate) use launcher_stack::launcher_state_path_from;

type Result<T, E = LauncherError> = std::result::Result<T, E>;

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

    #[snafu(display("creating the launcher state directory {} failed", path.display()))]
    CreateLauncherStateDirectory {
        source: std::io::Error,
        path: PathBuf,
    },

    #[snafu(display("reading the launcher state from {} failed", path.display()))]
    ReadLauncherState {
        source: std::io::Error,
        path: PathBuf,
    },

    #[snafu(display("parsing the launcher state from {} failed", path.display()))]
    ParseLauncherState {
        source: serde_json::Error,
        path: PathBuf,
    },

    #[snafu(display("serializing the launcher state failed"))]
    SerializeLauncherState { source: serde_json::Error },

    #[snafu(display("writing the launcher state to {} failed", path.display()))]
    WriteLauncherState {
        source: std::io::Error,
        path: PathBuf,
    },

    #[snafu(display("acquiring the launcher lock at {} failed", path.display()))]
    AcquireLauncherLock {
        source: std::io::Error,
        path: PathBuf,
    },

    #[snafu(display(
        "reading the launcher lock metadata from {} failed",
        path.display()
    ))]
    ReadLauncherLockMetadata {
        source: std::io::Error,
        path: PathBuf,
    },

    #[snafu(display("removing the launcher lock at {} failed", path.display()))]
    RemoveLauncherLock {
        source: std::io::Error,
        path: PathBuf,
    },

    #[snafu(display("timed out waiting for the launcher lock at {}", path.display()))]
    WaitForLauncherLock { path: PathBuf },

    #[snafu(display(
        "unable to determine a safe launcher state directory; set ACP_LAUNCHER_STATE_PATH"
    ))]
    MissingLauncherStateDirectory,

    #[snafu(display(
        "reading the launcher executable metadata from {} failed",
        path.display()
    ))]
    ReadLauncherExecutableMetadata {
        source: std::io::Error,
        path: PathBuf,
    },

    #[snafu(display(
        "reading the launcher executable modification time from {} failed",
        path.display()
    ))]
    ReadLauncherExecutableModifiedTime {
        source: std::io::Error,
        path: PathBuf,
    },
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct LauncherArgs {
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
    let mut stack = prepare_launcher_stack(
        &current_executable,
        &launcher_args,
        command_needs_backend(&launcher_args.cli_args),
        cli_server_url_is_explicit(&launcher_args.cli_args),
    )
    .await?;
    let cli_status = run_cli_foreground(
        &current_executable,
        launcher_args.cli_args,
        stack.backend_url(),
        stack.auth_token(),
    )
    .await?;

    stack.shutdown().await?;
    ensure_success("cli frontend", cli_status)
}

async fn run_cli_foreground(
    current_executable: &Path,
    cli_args: Vec<OsString>,
    backend_url: Option<&str>,
    auth_token: Option<&str>,
) -> Result<std::process::ExitStatus> {
    let mut envs = Vec::new();
    if let Some(backend_url) = backend_url {
        envs.push(("ACP_SERVER_URL", backend_url));
    }
    if let Some(auth_token) = auth_token {
        envs.push(("ACP_AUTH_TOKEN", auth_token));
    }

    spawn_foreground_role(current_executable, "cli frontend", "cli", cli_args, &envs).await
}

fn command_needs_backend(cli_args: &[OsString]) -> bool {
    let args = cli_args.iter().map(|arg| arg.to_str()).collect::<Vec<_>>();
    let is_help_or_version = args
        .iter()
        .any(|arg| matches!(arg, Some("-h" | "--help" | "-V" | "--version")));
    let is_session_list = matches!(args.as_slice(), [Some("session"), Some("list"), ..]);

    !is_help_or_version && !is_session_list
}

fn cli_server_url_is_explicit(cli_args: &[OsString]) -> bool {
    cli_args.iter().any(|arg| {
        arg.to_str()
            .is_some_and(|value| value == "--server-url" || value.starts_with("--server-url="))
    })
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
