use std::{
    env,
    ffi::OsString,
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    time::Duration,
};

use acp_app_support::{build_http_client_for_url, wait_for_health, wait_for_tcp_connect};
use serde::{Deserialize, Serialize};
use snafu::prelude::*;
use tokio::process::Child;

use crate::{
    CreateLauncherStateDirectorySnafu, LauncherArgs, ParseLauncherStateSnafu,
    ReadLauncherStateSnafu, Result, SerializeLauncherStateSnafu, WriteLauncherStateSnafu,
    launcher_process::{SpawnedService, spawn_background_role, terminate_child},
};

const LAUNCHER_STATE_ENV: &str = "ACP_LAUNCHER_STATE_PATH";
const LAUNCHER_STACK_EXIT_AFTER_ENV: &str = "ACP_LAUNCHER_STACK_EXIT_AFTER_MS";
const STACK_READY_ATTEMPTS: usize = 8;
const STACK_READY_DELAY: Duration = Duration::from_millis(75);
const STACK_READY_TIMEOUT: Duration = Duration::from_millis(250);
const STACK_LOCK_WAIT_ATTEMPTS: usize = 600;
const STACK_LOCK_WAIT_DELAY: Duration = Duration::from_millis(125);
const STACK_LOCK_STALE_AFTER: Duration = Duration::from_secs(90);

#[derive(Debug)]
pub(crate) struct LauncherStack {
    backend_url: Option<String>,
    bundled_mock: bool,
    ephemeral_children: Option<EphemeralChildren>,
}

#[derive(Debug)]
struct EphemeralChildren {
    backend: Child,
    mock: Option<Child>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct LauncherState {
    backend_url: String,
    #[serde(default)]
    mock_address: Option<String>,
}

#[derive(Debug)]
pub(crate) struct LauncherLock {
    path: PathBuf,
}

impl Drop for LauncherLock {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_file(&self.path)
            && error.kind() != ErrorKind::NotFound
        {
            warn_lock_cleanup_failure(&self.path, &error);
        }
    }
}

impl LauncherStack {
    fn direct() -> Self {
        Self {
            backend_url: None,
            bundled_mock: false,
            ephemeral_children: None,
        }
    }

    fn persistent(backend_url: String) -> Self {
        Self {
            backend_url: Some(backend_url),
            bundled_mock: true,
            ephemeral_children: None,
        }
    }

    fn ephemeral(backend: Child, mock: Option<Child>, backend_url: String) -> Self {
        Self {
            backend_url: Some(backend_url),
            bundled_mock: false,
            ephemeral_children: Some(EphemeralChildren { backend, mock }),
        }
    }

    pub(crate) fn backend_url(&self) -> Option<&str> {
        self.backend_url.as_deref()
    }

    pub(crate) fn bundled_mock(&self) -> bool {
        self.bundled_mock
    }

    pub(crate) async fn shutdown(&mut self) -> Result<()> {
        if let Some(children) = &mut self.ephemeral_children {
            let backend = &mut children.backend;
            terminate_child(backend, "web backend").await?;
            if let Some(mock) = children.mock.as_mut() {
                terminate_child(mock, "acp mock").await?;
            }
        }
        self.ephemeral_children = None;
        Ok(())
    }
}

pub(crate) async fn prepare_launcher_stack(
    current_executable: &Path,
    launcher_args: &LauncherArgs,
) -> Result<LauncherStack> {
    if !command_needs_backend(&launcher_args.cli_args)
        || cli_server_url_is_explicit(&launcher_args.cli_args)
    {
        return Ok(LauncherStack::direct());
    }

    if let Some(acp_server) = launcher_args.acp_server.clone() {
        return spawn_ephemeral_stack(current_executable, acp_server).await;
    }
    if env::var_os("ACP_SERVER_URL").is_some() {
        return Ok(LauncherStack::direct());
    }

    prepare_persistent_bundled_stack(current_executable).await
}

pub(crate) fn command_needs_backend(cli_args: &[OsString]) -> bool {
    !matches!(
        (
            cli_args.first().and_then(|arg| arg.to_str()),
            cli_args.get(1).and_then(|arg| arg.to_str()),
        ),
        (Some("session"), Some("list"))
    )
}

pub(crate) fn cli_server_url_is_explicit(cli_args: &[OsString]) -> bool {
    cli_args.iter().any(|arg| {
        arg.to_str()
            .is_some_and(|value| value == "--server-url" || value.starts_with("--server-url="))
    })
}

pub(crate) fn launcher_state_path_from(
    explicit_path: Option<OsString>,
    data_local_dir: Option<PathBuf>,
) -> PathBuf {
    if let Some(path) = explicit_path {
        return PathBuf::from(path);
    }

    if let Some(mut directory) = data_local_dir {
        directory.push("acp-orchestrator");
        directory.push("launcher-stack.json");
        return directory;
    }

    std::env::temp_dir().join("acp-orchestrator-launcher-stack.json")
}

pub(crate) fn launcher_lock_path_from(state_path: &Path) -> PathBuf {
    let mut path = state_path.as_os_str().to_os_string();
    path.push(".lock");
    PathBuf::from(path)
}

fn launcher_state_path() -> PathBuf {
    launcher_state_path_from(env::var_os(LAUNCHER_STATE_ENV), dirs::data_local_dir())
}

async fn spawn_ephemeral_stack(
    current_executable: &Path,
    acp_server: OsString,
) -> Result<LauncherStack> {
    let SpawnedService {
        child: backend,
        endpoint: backend_url,
    } = spawn_background_role(
        current_executable,
        "web backend",
        "backend",
        backend_role_args(acp_server),
        &[],
        true,
    )
    .await?;

    Ok(LauncherStack::ephemeral(backend, None, backend_url))
}

async fn prepare_persistent_bundled_stack(current_executable: &Path) -> Result<LauncherStack> {
    let state_path = launcher_state_path();
    prepare_persistent_bundled_stack_with_retry(
        current_executable,
        &state_path,
        STACK_LOCK_WAIT_ATTEMPTS,
        STACK_LOCK_WAIT_DELAY,
        STACK_LOCK_STALE_AFTER,
    )
    .await
}

async fn prepare_persistent_bundled_stack_with_retry(
    current_executable: &Path,
    state_path: &Path,
    lock_wait_attempts: usize,
    lock_wait_delay: Duration,
    lock_stale_after: Duration,
) -> Result<LauncherStack> {
    let lock_path = launcher_lock_path_from(state_path);

    for _ in 0..lock_wait_attempts {
        if let Some(state) = reusable_launcher_state(state_path).await? {
            return Ok(LauncherStack::persistent(state.backend_url));
        }

        if let Some(lock) = try_acquire_launcher_lock(&lock_path)? {
            return spawn_or_reuse_locked_stack(current_executable, state_path, lock).await;
        }
        if clear_stale_launcher_lock(&lock_path, lock_stale_after)? {
            continue;
        }
        tokio::time::sleep(lock_wait_delay).await;
    }

    if let Some(state) = reusable_launcher_state(state_path).await? {
        return Ok(LauncherStack::persistent(state.backend_url));
    }

    crate::WaitForLauncherLockSnafu { path: lock_path }.fail()
}

async fn spawn_or_reuse_locked_stack(
    current_executable: &Path,
    state_path: &Path,
    _lock: LauncherLock,
) -> Result<LauncherStack> {
    if let Some(state) = reusable_launcher_state(state_path).await? {
        return Ok(LauncherStack::persistent(state.backend_url));
    }

    let (mut mock, mut backend, state) =
        spawn_persistent_bundled_backend(current_executable).await?;
    persist_launcher_state_or_shutdown(state_path, &state, &mut backend, &mut mock).await?;
    let backend_url = state.backend_url.clone();

    drop(backend);
    drop(mock);
    Ok(LauncherStack::persistent(backend_url))
}

async fn reusable_launcher_state(state_path: &Path) -> Result<Option<LauncherState>> {
    let state = match load_launcher_state(state_path) {
        Ok(Some(state)) => state,
        Ok(None) => return Ok(None),
        Err(error @ crate::LauncherError::ParseLauncherState { .. }) => {
            warn_and_maybe_clear_invalid_launcher_state(state_path, &error);
            return Ok(None);
        }
        Err(error) => return Err(error),
    };

    let is_healthy = managed_stack_is_healthy(&state).await;

    if is_healthy {
        Ok(Some(state))
    } else {
        Ok(None)
    }
}

fn warn_and_maybe_clear_invalid_launcher_state(state_path: &Path, error: &crate::LauncherError) {
    if launcher_lock_path_from(state_path).exists() {
        return;
    }

    warn_invalid_launcher_state(state_path, error);
    if let Err(remove_error) = fs::remove_file(state_path)
        && remove_error.kind() != ErrorKind::NotFound
    {
        warn_invalid_launcher_state_cleanup_failure(state_path, &remove_error);
    }
}

fn warn_lock_cleanup_failure(path: &Path, error: &std::io::Error) {
    tracing::warn!(path = %path.display(), %error, "failed to remove the launcher lock file");
}

fn warn_invalid_launcher_state(path: &Path, error: &crate::LauncherError) {
    tracing::warn!(path = %path.display(), %error, "ignoring invalid launcher state");
}

fn warn_invalid_launcher_state_cleanup_failure(path: &Path, error: &std::io::Error) {
    tracing::warn!(path = %path.display(), %error, "failed to remove invalid launcher state");
}

pub(crate) fn try_acquire_launcher_lock(lock_path: &Path) -> Result<Option<LauncherLock>> {
    create_launcher_state_parent(lock_path)?;
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(lock_path)
    {
        Ok(_) => Ok(Some(LauncherLock {
            path: lock_path.to_path_buf(),
        })),
        Err(error) if error.kind() == ErrorKind::AlreadyExists => Ok(None),
        Err(source) => Err(crate::LauncherError::AcquireLauncherLock {
            source,
            path: lock_path.to_path_buf(),
        }),
    }
}

pub(crate) fn clear_stale_launcher_lock(lock_path: &Path, stale_after: Duration) -> Result<bool> {
    let metadata = match fs::metadata(lock_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(source) => {
            return Err(crate::LauncherError::ReadLauncherLockMetadata {
                source,
                path: lock_path.to_path_buf(),
            });
        }
    };
    let modified = metadata
        .modified()
        .context(crate::ReadLauncherLockMetadataSnafu {
            path: lock_path.to_path_buf(),
        })?;
    let is_stale = modified
        .elapsed()
        .ok()
        .is_some_and(|elapsed| elapsed >= stale_after);
    if !is_stale {
        return Ok(false);
    }

    match fs::remove_file(lock_path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(source) => Err(crate::LauncherError::RemoveLauncherLock {
            source,
            path: lock_path.to_path_buf(),
        }),
    }
}

async fn spawn_persistent_bundled_backend(
    current_executable: &Path,
) -> Result<(Child, Child, LauncherState)> {
    let SpawnedService {
        child: mut mock,
        endpoint: mock_address,
    } = spawn_background_role(
        current_executable,
        "acp mock",
        "mock",
        mock_role_args(),
        &[],
        false,
    )
    .await?;

    let backend = spawn_background_role(
        current_executable,
        "web backend",
        "backend",
        backend_role_args(OsString::from(&mock_address)),
        &[],
        false,
    )
    .await;
    match backend {
        Ok(SpawnedService {
            child: backend,
            endpoint,
        }) => Ok((
            mock,
            backend,
            LauncherState {
                backend_url: endpoint,
                mock_address: Some(mock_address),
            },
        )),
        Err(error) => {
            let _ = terminate_child(&mut mock, "acp mock").await;
            Err(error)
        }
    }
}

async fn persist_launcher_state_or_shutdown(
    state_path: &Path,
    state: &LauncherState,
    backend: &mut Child,
    mock: &mut Child,
) -> Result<()> {
    let save_result = save_launcher_state(state_path, state);
    if let Err(error) = save_result {
        let _ = terminate_child(backend, "web backend").await;
        let _ = terminate_child(mock, "acp mock").await;
        return Err(error);
    }
    Ok(())
}

fn backend_role_args(acp_server: OsString) -> Vec<OsString> {
    let mut args = vec![
        "--port".into(),
        "0".into(),
        "--acp-server".into(),
        acp_server,
    ];
    append_stack_exit_after_ms(&mut args);
    args
}

fn mock_role_args() -> Vec<OsString> {
    let mut args = vec!["--port".into(), "0".into()];
    append_stack_exit_after_ms(&mut args);
    args
}

fn append_stack_exit_after_ms(args: &mut Vec<OsString>) {
    if let Some(value) =
        env::var_os(LAUNCHER_STACK_EXIT_AFTER_ENV).filter(|value| !value.is_empty())
    {
        args.push("--exit-after-ms".into());
        args.push(value);
    }
}

fn load_launcher_state(path: &Path) -> Result<Option<LauncherState>> {
    if !path.exists() {
        return Ok(None);
    }

    let raw = fs::read_to_string(path).context(ReadLauncherStateSnafu {
        path: path.to_path_buf(),
    })?;
    let state = serde_json::from_str(&raw).context(ParseLauncherStateSnafu {
        path: path.to_path_buf(),
    })?;
    Ok(Some(state))
}

fn save_launcher_state(path: &Path, state: &LauncherState) -> Result<()> {
    create_launcher_state_parent(path)?;
    let serialized = serde_json::to_string_pretty(state).context(SerializeLauncherStateSnafu)?;
    fs::write(path, serialized).context(WriteLauncherStateSnafu {
        path: path.to_path_buf(),
    })?;
    Ok(())
}

fn create_launcher_state_parent(path: &Path) -> Result<()> {
    let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    else {
        return Ok(());
    };

    let parent = parent.to_path_buf();
    fs::create_dir_all(&parent).context(CreateLauncherStateDirectorySnafu { path: parent })?;
    Ok(())
}

async fn managed_stack_is_healthy(state: &LauncherState) -> bool {
    async {
        let mock_address = state.mock_address.as_deref()?;
        if wait_for_tcp_connect(mock_address, STACK_READY_ATTEMPTS, STACK_READY_DELAY)
            .await
            .is_err()
        {
            return Some(false);
        }

        let timeout = Some(STACK_READY_TIMEOUT);
        let client = build_http_client_for_url(&state.backend_url, timeout).ok()?;
        Some(
            wait_for_health(
                &client,
                &state.backend_url,
                STACK_READY_ATTEMPTS,
                STACK_READY_DELAY,
            )
            .await
            .is_ok(),
        )
    }
    .await
    .unwrap_or(false)
}

#[cfg(test)]
mod tests;
