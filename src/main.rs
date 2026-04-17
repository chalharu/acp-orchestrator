use std::{
    env,
    ffi::OsString,
    future::Future,
    path::{Path, PathBuf},
    time::Duration,
};

use acp_app_support::{
    BoxError, FrontendBundleAsset, build_http_client_for_url, frontend_bundle_exists, init_tracing,
    wait_for_http_success,
};
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

    #[snafu(display("building the web frontend failed: {message}"))]
    FrontendBuild { message: String },
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
    run_launcher(&current_executable, launcher_args).await
}

async fn run_launcher(current_executable: &Path, launcher_args: LauncherArgs) -> Result<()> {
    let needs_backend = launcher_args.web || command_needs_backend(&launcher_args.cli_args);
    let cli_server_url_explicit = cli_server_url_is_explicit(&launcher_args.cli_args);
    let frontend_dist =
        prepare_frontend_dist(&launcher_args, needs_backend, cli_server_url_explicit).await?;

    let mut stack = prepare_launcher_stack(
        current_executable,
        &launcher_args,
        needs_backend,
        cli_server_url_explicit,
        frontend_dist.as_deref(),
    )
    .await?;
    if launcher_args.web {
        return run_web_launcher(&mut stack).await;
    }
    run_cli_launcher(current_executable, launcher_args.cli_args, &mut stack).await
}

async fn prepare_frontend_dist(
    launcher_args: &LauncherArgs,
    needs_backend: bool,
    cli_server_url_explicit: bool,
) -> Result<Option<PathBuf>> {
    // For web mode, build the Leptos/WASM frontend before spawning the backend
    // so the backend can be given a valid --frontend-dist path.
    // Skip the build when an external backend is already provided via
    // ACP_SERVER_URL (direct stack) – in that case we can't configure its dist.
    if !should_prepare_frontend_dist(launcher_args, needs_backend, cli_server_url_explicit) {
        return Ok(None);
    }

    let dist = frontend_dist_path();
    ensure_frontend_built(&dist).await?;
    Ok(Some(dist))
}

fn should_prepare_frontend_dist(
    launcher_args: &LauncherArgs,
    needs_backend: bool,
    cli_server_url_explicit: bool,
) -> bool {
    launcher_args.web
        && needs_backend
        && !cli_server_url_explicit
        && env::var_os("ACP_SERVER_URL").is_none()
}

fn frontend_crate_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("crates")
        .join("acp-web-frontend")
}

/// Returns the Trunk dist directory for the web frontend.
/// The path is anchored to the repository root so `cargo run -- --web`
/// keeps working even when launched outside the repo root.
fn frontend_dist_path() -> PathBuf {
    frontend_crate_path().join("dist")
}

/// Ensures the Leptos/WASM frontend bundle exists in `dist`.
/// Runs `trunk build --release` in the frontend crate directory when the
/// compiled artefacts are absent.
async fn ensure_frontend_built(dist: &Path) -> Result<()> {
    if frontend_bundle_exists(dist, FrontendBundleAsset::JavaScript)
        && frontend_bundle_exists(dist, FrontendBundleAsset::Wasm)
    {
        return Ok(());
    }

    eprintln!(
        "web frontend not built yet – running `trunk build --release` in crates/acp-web-frontend/ …"
    );
    eprintln!("(re-run with `trunk build --release` in that directory to rebuild manually)");

    let status = tokio::process::Command::new("trunk")
        .args(["build", "--release"])
        .current_dir(frontend_crate_path())
        .status()
        .await
        .map_err(|source| {
            let detail = if source.kind() == std::io::ErrorKind::NotFound {
                "trunk not found – install it with `cargo install trunk`".to_string()
            } else {
                source.to_string()
            };
            LauncherError::FrontendBuild { message: detail }
        })?;

    if !status.success() {
        return Err(LauncherError::FrontendBuild {
            message: format!(
                "`trunk build --release` failed with exit code {:?}",
                status.code()
            ),
        });
    }

    Ok(())
}

async fn run_web_launcher(stack: &mut launcher_stack::LauncherStack) -> Result<()> {
    run_web_launcher_with_signal(stack, tokio::signal::ctrl_c()).await
}

async fn run_web_launcher_with_signal<F>(
    stack: &mut launcher_stack::LauncherStack,
    shutdown_signal: F,
) -> Result<()>
where
    F: Future<Output = std::io::Result<()>>,
{
    if let Err(error) = run_web_foreground(stack).await {
        cleanup_after_web_launch_error(stack).await;
        return Err(error);
    }
    if stack.is_ephemeral() {
        wait_for_web_shutdown_with_signal(stack, shutdown_signal).await?;
    }
    Ok(())
}

async fn cleanup_after_web_launch_error(stack: &mut launcher_stack::LauncherStack) {
    finish_web_launch_cleanup(stack.shutdown().await);
}

fn finish_web_launch_cleanup(shutdown_result: Result<(), LauncherError>) {
    if let Err(shutdown_error) = shutdown_result {
        tracing::warn!(%shutdown_error, "web launcher cleanup failed after an entrypoint error");
    }
}

async fn wait_for_web_shutdown_with_signal<F>(
    stack: &mut launcher_stack::LauncherStack,
    shutdown_signal: F,
) -> Result<()>
where
    F: Future<Output = std::io::Result<()>>,
{
    let wait_for_shutdown_signal = shutdown_signal.await.context(WaitForWebShutdownSignalSnafu);
    let shutdown_result = stack.shutdown().await;
    finish_web_shutdown(wait_for_shutdown_signal, shutdown_result)
}

fn finish_web_shutdown(
    wait_for_shutdown_signal: Result<(), LauncherError>,
    shutdown_result: Result<(), LauncherError>,
) -> Result<()> {
    if let Err(wait_error) = wait_for_shutdown_signal {
        if let Err(shutdown_error) = shutdown_result {
            tracing::warn!(%shutdown_error, "web launcher cleanup failed after waiting for the shutdown signal");
        }
        return Err(wait_error);
    }
    shutdown_result
}

async fn run_cli_launcher(
    current_executable: &Path,
    cli_args: Vec<OsString>,
    stack: &mut launcher_stack::LauncherStack,
) -> Result<()> {
    let cli_status = run_cli_foreground(
        current_executable,
        cli_args,
        stack.backend_url(),
        stack.auth_token(),
    )
    .await;
    let shutdown_result = stack.shutdown().await;
    finish_cli_launch(cli_status, shutdown_result)
}

fn finish_cli_launch(
    cli_status: Result<std::process::ExitStatus, LauncherError>,
    shutdown_result: Result<(), LauncherError>,
) -> Result<()> {
    match cli_status {
        Ok(status) => {
            shutdown_result?;
            ensure_success("cli frontend", status)
        }
        Err(error) => {
            if let Err(shutdown_error) = shutdown_result {
                tracing::warn!(
                    %shutdown_error,
                    "launcher cleanup failed after the CLI frontend returned an error"
                );
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
    run_web_foreground_with(stack, |app_url| open::that_detached(app_url)).await
}

async fn run_web_foreground_with<F, E>(
    stack: &launcher_stack::LauncherStack,
    open_browser: F,
) -> Result<()>
where
    F: FnOnce(&str) -> std::result::Result<(), E>,
    E: std::fmt::Display,
{
    let backend_url = web_backend_url(stack)?;
    let app_url = wait_for_web_entrypoint(&backend_url).await?;
    println!("opening browser: {app_url}");

    if let Err(error) = open_browser(&app_url) {
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
        if arg.as_os_str() == "--web" {
            launcher_args.web = true;
            continue;
        }
        if let Some(acp_server) = parse_acp_server_arg(&arg, &mut args)? {
            launcher_args.acp_server = Some(acp_server);
            continue;
        }
        launcher_args.cli_args.push(arg);
    }

    apply_default_cli_args(&mut launcher_args);
    Ok(launcher_args)
}

fn parse_acp_server_arg<I>(arg: &OsString, args: &mut I) -> Result<Option<OsString>>
where
    I: Iterator<Item = OsString>,
{
    if arg.as_os_str() == "--acp-server" {
        return next_acp_server_arg(args).map(Some);
    }
    let Some(value) = arg
        .to_str()
        .and_then(|value| value.strip_prefix("--acp-server="))
    else {
        return Ok(None);
    };
    validate_acp_server_arg(OsString::from(value)).map(Some)
}

fn next_acp_server_arg<I>(args: &mut I) -> Result<OsString>
where
    I: Iterator<Item = OsString>,
{
    let value = args.next().ok_or_else(|| MissingAcpServerSnafu.build())?;
    validate_acp_server_arg(value)
}

fn validate_acp_server_arg(value: OsString) -> Result<OsString> {
    if value.is_empty() {
        return MissingAcpServerSnafu.fail();
    }
    Ok(value)
}

fn apply_default_cli_args(launcher_args: &mut LauncherArgs) {
    if !launcher_args.web && launcher_args.cli_args.is_empty() {
        launcher_args.cli_args = vec!["chat".into(), "--new".into()];
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
