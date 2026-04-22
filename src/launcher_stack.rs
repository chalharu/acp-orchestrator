use std::{env, ffi::OsString, path::Path, time::Duration};

use tokio::process::Child;
use uuid::Uuid;

use crate::{
    LauncherArgs, Result,
    launcher_process::{SpawnedService, spawn_background_role, terminate_child},
};

mod lock;
mod state;

#[cfg(test)]
pub(crate) use self::lock::write_launcher_lock_owner;
pub(crate) use self::lock::{
    LauncherLock, clear_stale_launcher_lock, launcher_lock_path_from, try_acquire_launcher_lock,
};
use self::state::{
    LauncherIdentity, LauncherState, create_launcher_state_parent, current_launcher_identity,
    launcher_state_path, launcher_state_supports_requested_stack, load_launcher_state,
    managed_stack_is_healthy, persistent_launcher_state, persistent_stack_from_state,
    save_launcher_state, warn_and_maybe_clear_invalid_launcher_state,
};
#[cfg(test)]
pub(crate) use self::state::{
    launcher_state_path_from, path_to_string, socket_address_uses_loopback,
};

const LAUNCHER_STATE_ENV: &str = "ACP_LAUNCHER_STATE_PATH";
const LAUNCHER_STACK_EXIT_AFTER_ENV: &str = "ACP_LAUNCHER_STACK_EXIT_AFTER_MS";
const STACK_READY_ATTEMPTS: usize = 8;
const STACK_READY_DELAY: Duration = Duration::from_millis(75);
const STACK_READY_TIMEOUT: Duration = Duration::from_millis(250);
const STACK_LOCK_WAIT_ATTEMPTS: usize = 600;
const STACK_LOCK_WAIT_DELAY: Duration = Duration::from_millis(125);
const STACK_LOCK_STALE_AFTER: Duration = Duration::from_secs(30);
const BUNDLED_STARTUP_HINTS: bool = true;

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
    frontend_dist: Option<&Path>,
) -> Result<LauncherStack> {
    if !needs_backend || cli_server_url_explicit {
        return Ok(LauncherStack::direct());
    }

    if let Some(acp_server) = launcher_args.acp_server.clone() {
        return spawn_ephemeral_stack(current_executable, acp_server, frontend_dist).await;
    }
    if env::var_os("ACP_SERVER_URL").is_some() {
        return Ok(LauncherStack::direct());
    }

    prepare_persistent_bundled_stack(current_executable, frontend_dist).await
}

async fn spawn_ephemeral_stack(
    current_executable: &Path,
    acp_server: OsString,
    frontend_dist: Option<&Path>,
) -> Result<LauncherStack> {
    let auth_token = Uuid::new_v4().to_string();
    let SpawnedService {
        child: backend,
        endpoint: backend_url,
    } = spawn_background_role(
        current_executable,
        "web backend",
        "backend",
        backend_role_args(acp_server, false, frontend_dist),
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

async fn prepare_persistent_bundled_stack(
    current_executable: &Path,
    frontend_dist: Option<&Path>,
) -> Result<LauncherStack> {
    let state_path = launcher_state_path()?;
    let launcher_identity = current_launcher_identity(current_executable)?;
    prepare_persistent_bundled_stack_with_retry(
        current_executable,
        &state_path,
        &launcher_identity,
        STACK_LOCK_WAIT_ATTEMPTS,
        STACK_LOCK_WAIT_DELAY,
        STACK_LOCK_STALE_AFTER,
        frontend_dist,
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
    frontend_dist: Option<&Path>,
) -> Result<LauncherStack> {
    let lock_path = launcher_lock_path_from(state_path);

    for _ in 0..lock_wait_attempts {
        if let Some(stack) =
            reusable_persistent_stack(state_path, launcher_identity, frontend_dist).await?
        {
            return Ok(stack);
        }

        if let Some(lock) = try_acquire_launcher_lock(&lock_path)? {
            return spawn_or_reuse_locked_stack(
                current_executable,
                state_path,
                launcher_identity,
                lock,
                frontend_dist,
            )
            .await;
        }
        if clear_stale_launcher_lock(&lock_path, lock_stale_after)? {
            continue;
        }
        tokio::time::sleep(lock_wait_delay).await;
    }

    if let Some(stack) =
        reusable_persistent_stack(state_path, launcher_identity, frontend_dist).await?
    {
        return Ok(stack);
    }

    crate::WaitForLauncherLockSnafu { path: lock_path }.fail()
}

async fn spawn_or_reuse_locked_stack(
    current_executable: &Path,
    state_path: &Path,
    launcher_identity: &LauncherIdentity,
    _lock: LauncherLock,
    frontend_dist: Option<&Path>,
) -> Result<LauncherStack> {
    if let Some(stack) =
        reusable_persistent_stack(state_path, launcher_identity, frontend_dist).await?
    {
        return Ok(stack);
    }

    let (mut mock, mut backend, state) =
        spawn_persistent_bundled_backend(current_executable, launcher_identity, frontend_dist)
            .await?;
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
    frontend_dist: Option<&Path>,
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
    if !launcher_state_supports_requested_stack(&state, frontend_dist) {
        return Ok(None);
    }
    let is_healthy = managed_stack_is_healthy(&state).await;

    if is_healthy {
        Ok(Some(state))
    } else {
        Ok(None)
    }
}

async fn spawn_persistent_bundled_backend(
    current_executable: &Path,
    launcher_identity: &LauncherIdentity,
    frontend_dist: Option<&Path>,
) -> Result<(Child, Child, LauncherState)> {
    let auth_token = Uuid::new_v4().to_string();
    let SpawnedService {
        child: mut mock,
        endpoint: mock_address,
    } = spawn_background_role(
        current_executable,
        "acp mock",
        "mock",
        mock_role_args(BUNDLED_STARTUP_HINTS),
        &[],
        false,
    )
    .await?;

    let SpawnedService {
        child: backend,
        endpoint,
    } = match spawn_persistent_bundled_backend_service(
        current_executable,
        &mock_address,
        frontend_dist,
    )
    .await
    {
        Ok(service) => service,
        Err(error) => {
            let _ = terminate_child(&mut mock, "acp mock").await;
            return Err(error);
        }
    };

    Ok((
        mock,
        backend,
        persistent_launcher_state(
            endpoint,
            mock_address,
            frontend_dist,
            auth_token,
            launcher_identity,
        ),
    ))
}

async fn spawn_persistent_bundled_backend_service(
    current_executable: &Path,
    mock_address: &str,
    frontend_dist: Option<&Path>,
) -> Result<SpawnedService> {
    spawn_background_role(
        current_executable,
        "web backend",
        "backend",
        persistent_bundled_backend_role_args(mock_address, frontend_dist),
        &[],
        false,
    )
    .await
}

fn persistent_bundled_backend_role_args(
    mock_address: &str,
    frontend_dist: Option<&Path>,
) -> Vec<OsString> {
    backend_role_args(
        OsString::from(mock_address),
        BUNDLED_STARTUP_HINTS,
        frontend_dist,
    )
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

fn backend_role_args(
    acp_server: OsString,
    startup_hints: bool,
    frontend_dist: Option<&Path>,
) -> Vec<OsString> {
    let mut args = vec![
        "--port".into(),
        "0".into(),
        "--acp-server".into(),
        acp_server,
    ];
    if startup_hints {
        args.push("--startup-hints".into());
    }
    if let Some(dist) = frontend_dist {
        args.push("--frontend-dist".into());
        args.push(dist.as_os_str().to_owned());
    }
    append_stack_exit_after_ms(&mut args);
    args
}

async fn reusable_persistent_stack(
    state_path: &Path,
    launcher_identity: &LauncherIdentity,
    frontend_dist: Option<&Path>,
) -> Result<Option<LauncherStack>> {
    reusable_launcher_state(state_path, launcher_identity, frontend_dist)
        .await
        .map(|state| state.map(persistent_stack_from_state))
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

#[cfg(test)]
mod tests;
