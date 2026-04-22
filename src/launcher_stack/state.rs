use std::{
    env, fs,
    net::IpAddr,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use serde::{Deserialize, Serialize};
use snafu::prelude::*;

use crate::support::frontend::FRONTEND_JAVASCRIPT_ASSET_PATH;
use crate::support::http::{
    build_http_client_for_url, wait_for_health, wait_for_http_success, wait_for_tcp_connect,
};
use crate::{
    CreateLauncherStateDirectorySnafu, MissingLauncherStateDirectorySnafu, ParseLauncherStateSnafu,
    ReadLauncherExecutableMetadataSnafu, ReadLauncherExecutableModifiedTimeSnafu,
    ReadLauncherStateSnafu, Result, SerializeLauncherStateSnafu, WriteLauncherStateSnafu,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct LauncherState {
    pub(super) backend_url: String,
    #[serde(default)]
    pub(super) mock_address: Option<String>,
    #[serde(default)]
    pub(super) frontend_dist: Option<String>,
    #[serde(default)]
    pub(super) startup_hints: bool,
    pub(super) auth_token: String,
    pub(super) launcher_identity: LauncherIdentity,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct LauncherIdentity {
    pub(super) executable_path: String,
    pub(super) build_fingerprint: String,
}

pub(crate) fn launcher_state_path_from(
    explicit_path: Option<std::ffi::OsString>,
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

pub(super) fn launcher_state_path() -> Result<PathBuf> {
    launcher_state_path_from(
        env::var_os(super::LAUNCHER_STATE_ENV),
        dirs::data_local_dir(),
        dirs::home_dir(),
    )
}

pub(super) fn current_launcher_identity(current_executable: &Path) -> Result<LauncherIdentity> {
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

pub(super) fn persistent_launcher_state(
    endpoint: String,
    mock_address: String,
    frontend_dist: Option<&Path>,
    auth_token: String,
    launcher_identity: &LauncherIdentity,
) -> LauncherState {
    LauncherState {
        backend_url: endpoint,
        mock_address: Some(mock_address),
        frontend_dist: frontend_dist.map(path_to_string),
        startup_hints: super::BUNDLED_STARTUP_HINTS,
        auth_token,
        launcher_identity: launcher_identity.clone(),
    }
}

pub(super) fn persistent_stack_from_state(state: LauncherState) -> super::LauncherStack {
    super::LauncherStack::persistent(state.backend_url, state.auth_token)
}

pub(super) fn launcher_state_supports_requested_stack(
    state: &LauncherState,
    requested_frontend_dist: Option<&Path>,
) -> bool {
    let frontend_matches = match requested_frontend_dist.map(path_to_string) {
        Some(requested) => state.frontend_dist.as_deref() == Some(requested.as_str()),
        None => true,
    };

    frontend_matches && state.startup_hints == super::BUNDLED_STARTUP_HINTS
}

pub(crate) fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

pub(super) fn load_launcher_state(path: &Path) -> Result<Option<LauncherState>> {
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

pub(super) fn save_launcher_state(path: &Path, state: &LauncherState) -> Result<()> {
    super::create_launcher_state_parent(path)?;
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
    use std::io::Write as _;
    file.write_all(serialized.as_bytes())
        .context(WriteLauncherStateSnafu {
            path: path.to_path_buf(),
        })?;
    Ok(())
}

pub(super) async fn managed_stack_is_healthy(state: &LauncherState) -> bool {
    async {
        if state.auth_token.is_empty() || !launcher_state_endpoints_are_loopback(state) {
            return Some(false);
        }
        let mock_address = state.mock_address.as_deref()?;
        if wait_for_tcp_connect(
            mock_address,
            super::STACK_READY_ATTEMPTS,
            super::STACK_READY_DELAY,
        )
        .await
        .is_err()
        {
            return Some(false);
        }

        let timeout = Some(super::STACK_READY_TIMEOUT);
        let client = build_http_client_for_url(&state.backend_url, timeout).ok()?;
        let backend_ready = wait_for_health(
            &client,
            &state.backend_url,
            super::STACK_READY_ATTEMPTS,
            super::STACK_READY_DELAY,
        )
        .await
        .is_ok();
        let frontend_ready = if state.frontend_dist.is_some() {
            let frontend_asset_url = format!(
                "{}{}",
                state.backend_url.trim_end_matches('/'),
                FRONTEND_JAVASCRIPT_ASSET_PATH
            );
            wait_for_http_success(
                &client,
                &frontend_asset_url,
                super::STACK_READY_ATTEMPTS,
                super::STACK_READY_DELAY,
                "web frontend asset",
            )
            .await
            .is_ok()
        } else {
            true
        };
        Some(backend_ready && frontend_ready)
    }
    .await
    .unwrap_or(false)
}

pub(super) fn warn_and_maybe_clear_invalid_launcher_state(
    state_path: &Path,
    error: &crate::LauncherError,
) {
    if super::launcher_lock_path_from(state_path).exists() {
        return;
    }

    warn_invalid_launcher_state(state_path, error);
    if let Err(remove_error) = fs::remove_file(state_path)
        && remove_error.kind() != std::io::ErrorKind::NotFound
    {
        warn_invalid_launcher_state_cleanup_failure(state_path, &remove_error);
    }
}

fn warn_invalid_launcher_state(path: &Path, error: &crate::LauncherError) {
    tracing::warn!(path = %path.display(), %error, "ignoring invalid launcher state");
}

fn warn_invalid_launcher_state_cleanup_failure(path: &Path, error: &std::io::Error) {
    tracing::warn!(path = %path.display(), %error, "failed to remove invalid launcher state");
}

pub(super) fn create_launcher_state_parent(path: &Path) -> Result<()> {
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

pub(crate) fn socket_address_uses_loopback(address: &str) -> bool {
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
