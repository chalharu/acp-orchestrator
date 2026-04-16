use std::{
    env,
    ffi::OsString,
    path::{Path, PathBuf},
    time::Duration,
};

use acp_app_support::{BoxError, build_http_client_for_url, init_tracing, wait_for_http_success};
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

    #[snafu(display("missing a backend URL for web launch"))]
    MissingBackendUrl,

    #[snafu(display("running the cli child failed: {message}"))]
    RunCli { message: String },

    #[snafu(display("running the mock child failed: {message}"))]
    RunMock { message: String },

    #[snafu(display("running the backend child failed: {message}"))]
    RunBackend { message: String },

    #[snafu(display("building the web launch client failed"))]
    BuildWebClient { source: reqwest::Error },

    #[snafu(display("waiting for the web browser entrypoint failed"))]
    WaitForWebEntryPoint { source: BoxError },

    #[snafu(display("waiting for the web launcher shutdown signal failed"))]
    WaitForWebShutdownSignal { source: std::io::Error },

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
    web: bool,
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
        launcher_args.web || command_needs_backend(&launcher_args.cli_args),
        cli_server_url_is_explicit(&launcher_args.cli_args),
    )
    .await?;
    if launcher_args.web {
        let result = run_web_foreground(&stack).await;
        if result.is_err() {
            if let Err(shutdown_error) = stack.shutdown().await {
                tracing::warn!(%shutdown_error, "web launcher cleanup failed after an entrypoint error");
            }
            return result;
        }
        if stack.is_ephemeral() {
            let wait_for_shutdown_signal = tokio::signal::ctrl_c()
                .await
                .context(WaitForWebShutdownSignalSnafu);
            let shutdown_result = stack.shutdown().await;
            if let Err(wait_error) = wait_for_shutdown_signal {
                if let Err(shutdown_error) = shutdown_result {
                    tracing::warn!(%shutdown_error, "web launcher cleanup failed after waiting for the shutdown signal");
                }
                return Err(wait_error);
            }
            shutdown_result?;
        }
        return Ok(());
    }
    let cli_status = run_cli_foreground(
        &current_executable,
        launcher_args.cli_args,
        stack.backend_url(),
        stack.auth_token(),
    )
    .await;
    let shutdown_result = stack.shutdown().await;
    match cli_status {
        Ok(status) => {
            shutdown_result?;
            ensure_success("cli frontend", status)
        }
        Err(error) => {
            if let Err(shutdown_error) = shutdown_result {
                tracing::warn!(%shutdown_error, "launcher cleanup failed after the CLI frontend returned an error");
            }
            Err(error)
        }
    }
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

async fn run_web_foreground(stack: &launcher_stack::LauncherStack) -> Result<()> {
    let backend_url = web_backend_url(stack)?;
    let app_url = wait_for_web_entrypoint(&backend_url).await?;
    println!("opening browser: {app_url}");

    if let Err(error) = open::that_detached(&app_url) {
        eprintln!("failed to open the browser automatically: {error}");
    }

    Ok(())
}

fn web_backend_url(stack: &launcher_stack::LauncherStack) -> Result<String> {
    if let Some(backend_url) = stack.backend_url() {
        return Ok(backend_url.to_string());
    }

    env::var("ACP_SERVER_URL")
        .ok()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| MissingBackendUrlSnafu.build())
}

async fn wait_for_web_entrypoint(backend_url: &str) -> Result<String> {
    const WEB_READY_ATTEMPTS: usize = 50;
    const WEB_READY_DELAY: Duration = Duration::from_millis(100);
    const WEB_READY_TIMEOUT: Duration = Duration::from_millis(500);

    let client = build_http_client_for_url(backend_url, Some(WEB_READY_TIMEOUT))
        .context(BuildWebClientSnafu)?;
    let app_url = format!("{}/app/", backend_url.trim_end_matches('/'));
    wait_for_http_success(
        &client,
        &app_url,
        WEB_READY_ATTEMPTS,
        WEB_READY_DELAY,
        "browser entrypoint",
    )
    .await
    .map_err(|source| LauncherError::WaitForWebEntryPoint { source })?;
    Ok(app_url)
}
fn command_needs_backend(cli_args: &[OsString]) -> bool {
    let args = cli_args.iter().map(|arg| arg.to_str()).collect::<Vec<_>>();
    let is_help_or_version = args
        .iter()
        .any(|arg| matches!(arg, Some("-h" | "--help" | "-V" | "--version")));

    !is_help_or_version
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
        } else if arg.as_os_str() == "--web" {
            launcher_args.web = true;
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

    if !launcher_args.web && launcher_args.cli_args.is_empty() {
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
