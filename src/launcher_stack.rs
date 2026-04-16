use std::{
    env,
    ffi::OsString,
    fs,
    io::ErrorKind,
    io::Write,
    net::IpAddr,
    path::{Path, PathBuf},
    time::{Duration, UNIX_EPOCH},
};

use acp_app_support::{build_http_client_for_url, wait_for_health, wait_for_tcp_connect};
use serde::{Deserialize, Serialize};
use snafu::prelude::*;
use tokio::process::Child;
use uuid::Uuid;

use crate::{
    CreateLauncherStateDirectorySnafu, LauncherArgs, MissingLauncherStateDirectorySnafu,
    ParseLauncherStateSnafu, ReadLauncherExecutableMetadataSnafu,
    ReadLauncherExecutableModifiedTimeSnafu, ReadLauncherStateSnafu, Result,
    SerializeLauncherStateSnafu, WriteLauncherStateSnafu,
    launcher_process::{SpawnedService, spawn_background_role, terminate_child},
};

const LAUNCHER_STATE_ENV: &str = "ACP_LAUNCHER_STATE_PATH";
const LAUNCHER_STACK_EXIT_AFTER_ENV: &str = "ACP_LAUNCHER_STACK_EXIT_AFTER_MS";
const STACK_READY_ATTEMPTS: usize = 8;
const STACK_READY_DELAY: Duration = Duration::from_millis(75);
const STACK_READY_TIMEOUT: Duration = Duration::from_millis(250);
const STACK_LOCK_WAIT_ATTEMPTS: usize = 600;
const STACK_LOCK_WAIT_DELAY: Duration = Duration::from_millis(125);
const STACK_LOCK_STALE_AFTER: Duration = Duration::from_secs(30);

#[derive(Debug)]
pub(crate) struct LauncherStack {
    backend_url: Option<String>,
    auth_token: Option<String>,
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
    auth_token: String,
    launcher_identity: LauncherIdentity,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct LauncherIdentity {
    executable_path: String,
    build_fingerprint: String,
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
    pub(crate) fn direct() -> Self {
        Self {
            backend_url: None,
            auth_token: None,
            ephemeral_children: None,
        }
    }

    pub(crate) fn persistent(backend_url: String, auth_token: String) -> Self {
        Self {
            backend_url: Some(backend_url),
            auth_token: Some(auth_token),
            ephemeral_children: None,
        }
    }

    pub(crate) fn ephemeral(
        backend: Child,
        mock: Option<Child>,
        backend_url: String,
        auth_token: String,
    ) -> Self {
        Self {
            backend_url: Some(backend_url),
            auth_token: Some(auth_token),
            ephemeral_children: Some(EphemeralChildren { backend, mock }),
        }
    }

    pub(crate) fn backend_url(&self) -> Option<&str> {
        self.backend_url.as_deref()
    }

    pub(crate) fn auth_token(&self) -> Option<&str> {
        self.auth_token.as_deref()
    }

    pub(crate) fn is_ephemeral(&self) -> bool {
        self.ephemeral_children.is_some()
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
        self.auth_token = None;
        Ok(())
    }
}

pub(crate) async fn prepare_launcher_stack(
    current_executable: &Path,
    launcher_args: &LauncherArgs,
    needs_backend: bool,
    cli_server_url_explicit: bool,
) -> Result<LauncherStack> {
    if !needs_backend || cli_server_url_explicit {
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

pub(crate) fn launcher_state_path_from(
    explicit_path: Option<OsString>,
    data_local_dir: Option<PathBuf>,
    home_dir: Option<PathBuf>,
) -> Result<PathBuf> {
    if let Some(path) = explicit_path {
        return Ok(PathBuf::from(path));
    }

    if let Some(mut directory) = data_local_dir {
        directory.push("acp-orchestrator");
        directory.push("launcher-stack.json");
        return Ok(directory);
    }

    if let Some(mut directory) = home_dir {
        directory.push(".acp-orchestrator");
        directory.push("launcher-stack.json");
        return Ok(directory);
    }

    MissingLauncherStateDirectorySnafu.fail()
}

pub(crate) fn launcher_lock_path_from(state_path: &Path) -> PathBuf {
    let mut path = state_path.as_os_str().to_os_string();
    path.push(".lock");
    PathBuf::from(path)
}

fn launcher_state_path() -> Result<PathBuf> {
    launcher_state_path_from(
        env::var_os(LAUNCHER_STATE_ENV),
        dirs::data_local_dir(),
        dirs::home_dir(),
    )
}

fn current_launcher_identity(current_executable: &Path) -> Result<LauncherIdentity> {
    let metadata =
        fs::metadata(current_executable).context(ReadLauncherExecutableMetadataSnafu {
            path: current_executable.to_path_buf(),
        })?;
    let modified = metadata
        .modified()
        .context(ReadLauncherExecutableModifiedTimeSnafu {
            path: current_executable.to_path_buf(),
        })?
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();

    Ok(LauncherIdentity {
        executable_path: current_executable.display().to_string(),
        build_fingerprint: format!(
            "{}:{}:{}",
            metadata.len(),
            modified.as_secs(),
            modified.subsec_nanos()
        ),
    })
}

async fn spawn_ephemeral_stack(
    current_executable: &Path,
    acp_server: OsString,
) -> Result<LauncherStack> {
    let auth_token = Uuid::new_v4().to_string();
    let SpawnedService {
        child: backend,
        endpoint: backend_url,
    } = spawn_background_role(
        current_executable,
        "web backend",
        "backend",
        backend_role_args(acp_server, false),
        &[],
        true,
    )
    .await?;

    Ok(LauncherStack::ephemeral(
        backend,
        None,
        backend_url,
        auth_token,
    ))
}

async fn prepare_persistent_bundled_stack(current_executable: &Path) -> Result<LauncherStack> {
    let state_path = launcher_state_path()?;
    let launcher_identity = current_launcher_identity(current_executable)?;
    prepare_persistent_bundled_stack_with_retry(
        current_executable,
        &state_path,
        &launcher_identity,
        STACK_LOCK_WAIT_ATTEMPTS,
        STACK_LOCK_WAIT_DELAY,
        STACK_LOCK_STALE_AFTER,
    )
    .await
}

async fn prepare_persistent_bundled_stack_with_retry(
    current_executable: &Path,
    state_path: &Path,
    launcher_identity: &LauncherIdentity,
    lock_wait_attempts: usize,
    lock_wait_delay: Duration,
    lock_stale_after: Duration,
) -> Result<LauncherStack> {
    let lock_path = launcher_lock_path_from(state_path);

    for _ in 0..lock_wait_attempts {
        if let Some(state) = reusable_launcher_state(state_path, launcher_identity).await? {
            return Ok(LauncherStack::persistent(
                state.backend_url,
                state.auth_token,
            ));
        }

        if let Some(lock) = try_acquire_launcher_lock(&lock_path)? {
            return spawn_or_reuse_locked_stack(
                current_executable,
                state_path,
                launcher_identity,
                lock,
            )
            .await;
        }
        if clear_stale_launcher_lock(&lock_path, lock_stale_after)? {
            continue;
        }
        tokio::time::sleep(lock_wait_delay).await;
    }

    if let Some(state) = reusable_launcher_state(state_path, launcher_identity).await? {
        return Ok(LauncherStack::persistent(
            state.backend_url,
            state.auth_token,
        ));
    }

    crate::WaitForLauncherLockSnafu { path: lock_path }.fail()
}

async fn spawn_or_reuse_locked_stack(
    current_executable: &Path,
    state_path: &Path,
    launcher_identity: &LauncherIdentity,
    _lock: LauncherLock,
) -> Result<LauncherStack> {
    if let Some(state) = reusable_launcher_state(state_path, launcher_identity).await? {
        return Ok(LauncherStack::persistent(
            state.backend_url,
            state.auth_token,
        ));
    }

    let (mut mock, mut backend, state) =
        spawn_persistent_bundled_backend(current_executable, launcher_identity).await?;
    persist_launcher_state_or_shutdown(state_path, &state, &mut backend, &mut mock).await?;
    let backend_url = state.backend_url.clone();
    let auth_token = state.auth_token.clone();

    drop(backend);
    drop(mock);
    Ok(LauncherStack::persistent(backend_url, auth_token))
}

async fn reusable_launcher_state(
    state_path: &Path,
    launcher_identity: &LauncherIdentity,
) -> Result<Option<LauncherState>> {
    let state = match load_launcher_state(state_path) {
        Ok(Some(state)) => state,
        Ok(None) => return Ok(None),
        Err(error @ crate::LauncherError::ParseLauncherState { .. }) => {
            warn_and_maybe_clear_invalid_launcher_state(state_path, &error);
            return Ok(None);
        }
        Err(error) => return Err(error),
    };

    if state.launcher_identity != *launcher_identity {
        return Ok(None);
    }
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
    launcher_identity: &LauncherIdentity,
) -> Result<(Child, Child, LauncherState)> {
    let auth_token = Uuid::new_v4().to_string();
    let SpawnedService {
        child: mut mock,
        endpoint: mock_address,
    } = spawn_background_role(
        current_executable,
        "acp mock",
        "mock",
        mock_role_args(true),
        &[],
        false,
    )
    .await?;

    let backend = spawn_background_role(
        current_executable,
        "web backend",
        "backend",
        backend_role_args(OsString::from(&mock_address), true),
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
                auth_token,
                launcher_identity: launcher_identity.clone(),
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

fn backend_role_args(acp_server: OsString, startup_hints: bool) -> Vec<OsString> {
    let mut args = vec![
        "--port".into(),
        "0".into(),
        "--acp-server".into(),
        acp_server,
    ];
    if startup_hints {
        args.push("--startup-hints".into());
    }
    append_stack_exit_after_ms(&mut args);
    args
}

fn mock_role_args(startup_hints: bool) -> Vec<OsString> {
    let mut args = vec!["--port".into(), "0".into()];
    if startup_hints {
        args.push("--startup-hints".into());
    }
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
    let mut file = fs::File::create(path).context(WriteLauncherStateSnafu {
        path: path.to_path_buf(),
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let path_buf = path.to_path_buf();
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .context(WriteLauncherStateSnafu { path: path_buf })?;
    }
    file.write_all(serialized.as_bytes())
        .context(WriteLauncherStateSnafu {
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
    let parent = path.parent().expect("validated parent should exist");
    secure_launcher_state_parent_permissions(parent)?;
    Ok(())
}

async fn managed_stack_is_healthy(state: &LauncherState) -> bool {
    async {
        if state.auth_token.is_empty() || !launcher_state_endpoints_are_loopback(state) {
            return Some(false);
        }
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

#[cfg(unix)]
fn secure_launcher_state_parent_permissions(parent: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    if parent
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| matches!(name, "acp-orchestrator" | ".acp-orchestrator"))
    {
        let parent_path = parent.to_path_buf();
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
            .context(CreateLauncherStateDirectorySnafu { path: parent_path })?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn secure_launcher_state_parent_permissions(_parent: &Path) -> Result<()> {
    Ok(())
}

fn launcher_state_endpoints_are_loopback(state: &LauncherState) -> bool {
    let Some(mock_address) = state.mock_address.as_deref() else {
        return false;
    };

    socket_address_uses_loopback(mock_address) && backend_url_uses_loopback(&state.backend_url)
}

fn socket_address_uses_loopback(address: &str) -> bool {
    let host = if let Some(rest) = address.strip_prefix('[') {
        rest.split_once(']').map(|(host, _)| host)
    } else {
        address.rsplit_once(':').map(|(host, _)| host)
    };
    host.is_some_and(host_uses_loopback)
}

fn backend_url_uses_loopback(url: &str) -> bool {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_string))
        .as_deref()
        .is_some_and(host_uses_loopback)
}

fn host_uses_loopback(host: &str) -> bool {
    let host = host.trim_matches(|character| character == '[' || character == ']');

    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

#[cfg(test)]
mod tests;
