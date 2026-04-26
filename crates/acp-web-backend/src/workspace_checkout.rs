#[cfg(test)]
use std::cell::RefCell;
#[cfg(test)]
use std::ffi::OsString;
use std::{
    env,
    ffi::OsStr,
    fs,
    path::{Component, Path, PathBuf},
    process::Command,
    sync::Arc,
};

use async_trait::async_trait;
use reqwest::Url;

use crate::workspace_records::WorkspaceRecord;

const CHECKOUTS_DIR_NAME: &str = "session-checkouts";

#[cfg(test)]
#[derive(Clone)]
struct TestGitCommand {
    program: PathBuf,
    prefix_args: Vec<OsString>,
}

#[cfg(test)]
thread_local! {
    static TEST_GIT_BIN_OVERRIDE: RefCell<Option<TestGitCommand>> = const { RefCell::new(None) };
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedWorkspaceCheckout {
    pub checkout_relpath: String,
    pub checkout_ref: Option<String>,
    pub checkout_commit_sha: Option<String>,
    pub working_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceCheckoutError {
    Validation(String),
    Io(String),
    Git(String),
}

impl WorkspaceCheckoutError {
    pub fn message(&self) -> &str {
        match self {
            Self::Validation(message) | Self::Io(message) | Self::Git(message) => message,
        }
    }
}

impl std::fmt::Display for WorkspaceCheckoutError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message())
    }
}

impl std::error::Error for WorkspaceCheckoutError {}

#[async_trait]
pub trait WorkspaceCheckoutManager: Send + Sync {
    async fn prepare_checkout(
        &self,
        workspace: &WorkspaceRecord,
        session_id: &str,
        checkout_ref_override: Option<&str>,
    ) -> Result<PreparedWorkspaceCheckout, WorkspaceCheckoutError>;

    fn resolve_checkout_path(&self, _checkout_relpath: &str) -> Option<PathBuf> {
        None
    }
}

pub type DynWorkspaceCheckoutManager = Arc<dyn WorkspaceCheckoutManager>;

#[derive(Debug, Clone)]
pub struct FsWorkspaceCheckoutManager {
    state_dir: PathBuf,
}

impl FsWorkspaceCheckoutManager {
    pub fn new(state_dir: PathBuf) -> Self {
        Self { state_dir }
    }

    fn checkout_relpath(session_id: &str) -> String {
        format!("{CHECKOUTS_DIR_NAME}/{session_id}")
    }

    fn checkout_path(&self, session_id: &str) -> PathBuf {
        self.state_dir.join(Self::checkout_relpath(session_id))
    }

    fn resolved_checkout_path(&self, checkout_relpath: &str) -> Option<PathBuf> {
        let relpath = Path::new(checkout_relpath);
        let mut components = relpath.components();
        match components.next() {
            Some(Component::Normal(component)) if component == OsStr::new(CHECKOUTS_DIR_NAME) => {}
            _ => return None,
        }
        if components.any(|component| !matches!(component, Component::Normal(_))) {
            return None;
        }
        Some(self.state_dir.join(relpath))
    }

    fn prepare_checkout_sync(
        &self,
        workspace: &WorkspaceRecord,
        session_id: &str,
        checkout_ref_override: Option<&str>,
    ) -> Result<PreparedWorkspaceCheckout, WorkspaceCheckoutError> {
        let validated_override = validate_checkout_ref(checkout_ref_override)?;
        let checkout_relpath = Self::checkout_relpath(session_id);
        let checkout_path = self.checkout_path(session_id);
        let checkout_parent = checkout_parent_dir(&checkout_path)?;
        fs::create_dir_all(checkout_parent).map_err(|error| {
            WorkspaceCheckoutError::Io(format!("creating checkout root failed: {error}"))
        })?;
        if checkout_path.exists() {
            fs::remove_dir_all(&checkout_path).map_err(|error| {
                WorkspaceCheckoutError::Io(format!(
                    "clearing stale checkout directory failed: {error}"
                ))
            })?;
        }

        let prepared = match workspace.upstream_url.as_deref() {
            Some(upstream_url) => self.clone_https_workspace(
                upstream_url,
                workspace.default_ref.as_deref(),
                validated_override.as_deref(),
                &checkout_path,
                checkout_relpath,
            ),
            None => self.clone_local_workspace(
                workspace.default_ref.as_deref(),
                validated_override.as_deref(),
                &checkout_path,
                checkout_relpath,
            ),
        };

        if prepared.is_err() && checkout_path.exists() {
            let _ = fs::remove_dir_all(&checkout_path);
        }

        prepared
    }

    fn clone_https_workspace(
        &self,
        upstream_url: &str,
        default_ref: Option<&str>,
        override_ref: Option<&str>,
        checkout_path: &Path,
        checkout_relpath: String,
    ) -> Result<PreparedWorkspaceCheckout, WorkspaceCheckoutError> {
        validate_https_upstream_url(upstream_url)?;
        let resolved_ref =
            resolve_https_checkout_ref(upstream_url, default_ref, override_ref, &self.state_dir)?;
        initialize_https_checkout(checkout_path, upstream_url, &self.state_dir)?;
        fetch_https_checkout(checkout_path, resolved_ref.as_deref(), &self.state_dir)?;
        checkout_fetch_head(checkout_path, &self.state_dir, GitMode::Https)?;
        let checkout_commit_sha =
            checkout_head_commit(checkout_path, &self.state_dir, GitMode::Https)?;
        Ok(build_prepared_checkout(
            checkout_relpath,
            resolved_ref,
            checkout_commit_sha,
            checkout_path,
        ))
    }

    fn clone_local_workspace(
        &self,
        default_ref: Option<&str>,
        override_ref: Option<&str>,
        checkout_path: &Path,
        checkout_relpath: String,
    ) -> Result<PreparedWorkspaceCheckout, WorkspaceCheckoutError> {
        let source_root = local_source_root(&self.state_dir)?;
        let resolved_ref =
            resolve_local_checkout_ref(&source_root, default_ref, override_ref, &self.state_dir)?;
        clone_local_repository(&source_root, checkout_path, &self.state_dir)?;
        checkout_local_ref_if_needed(checkout_path, resolved_ref.as_deref(), &self.state_dir)?;
        Ok(build_prepared_checkout(
            checkout_relpath,
            resolved_ref,
            checkout_head_commit(checkout_path, &self.state_dir, GitMode::Local)?,
            checkout_path,
        ))
    }
}

#[async_trait]
impl WorkspaceCheckoutManager for FsWorkspaceCheckoutManager {
    async fn prepare_checkout(
        &self,
        workspace: &WorkspaceRecord,
        session_id: &str,
        checkout_ref_override: Option<&str>,
    ) -> Result<PreparedWorkspaceCheckout, WorkspaceCheckoutError> {
        let manager = self.clone();
        let workspace = workspace.clone();
        let session_id = session_id.to_string();
        let checkout_ref_override = checkout_ref_override.map(str::to_string);

        await_checkout_task(tokio::task::spawn_blocking(move || {
            manager.prepare_checkout_sync(&workspace, &session_id, checkout_ref_override.as_deref())
        }))
        .await
    }

    fn resolve_checkout_path(&self, checkout_relpath: &str) -> Option<PathBuf> {
        self.resolved_checkout_path(checkout_relpath)
    }
}

async fn await_checkout_task(
    handle: tokio::task::JoinHandle<Result<PreparedWorkspaceCheckout, WorkspaceCheckoutError>>,
) -> Result<PreparedWorkspaceCheckout, WorkspaceCheckoutError> {
    handle.await.map_err(|error| {
        WorkspaceCheckoutError::Io(format!("joining checkout task failed: {error}"))
    })?
}

fn checkout_parent_dir(checkout_path: &Path) -> Result<&Path, WorkspaceCheckoutError> {
    checkout_path.parent().ok_or_else(|| {
        WorkspaceCheckoutError::Io("session checkout path must have a parent directory".to_string())
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GitMode {
    Https,
    Local,
}

fn validate_checkout_ref(
    checkout_ref: Option<&str>,
) -> Result<Option<String>, WorkspaceCheckoutError> {
    let Some(checkout_ref) = checkout_ref else {
        return Ok(None);
    };
    let checkout_ref = checkout_ref.trim();
    if checkout_ref.is_empty() {
        return Err(WorkspaceCheckoutError::Validation(
            "checkout_ref must not be empty".to_string(),
        ));
    }
    if checkout_ref.chars().any(char::is_whitespace)
        || checkout_ref.starts_with('-')
        || checkout_ref.ends_with('.')
        || checkout_ref.starts_with('/')
        || checkout_ref.ends_with('/')
        || checkout_ref.contains("..")
        || checkout_ref.contains('@')
        || checkout_ref.contains('\\')
    {
        return Err(WorkspaceCheckoutError::Validation(
            "checkout_ref is invalid".to_string(),
        ));
    }
    Ok(Some(checkout_ref.to_string()))
}

fn validate_https_upstream_url(upstream_url: &str) -> Result<(), WorkspaceCheckoutError> {
    let parsed = Url::parse(upstream_url).map_err(|_| {
        WorkspaceCheckoutError::Validation("upstream_url must be a valid URL".to_string())
    })?;
    if parsed.scheme() != "https" {
        return Err(WorkspaceCheckoutError::Validation(
            "upstream_url must use https".to_string(),
        ));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(WorkspaceCheckoutError::Validation(
            "upstream_url must not embed credentials".to_string(),
        ));
    }
    Ok(())
}

fn resolve_remote_head_ref(
    upstream_url: &str,
    state_dir: &Path,
) -> Result<Option<String>, WorkspaceCheckoutError> {
    let output = run_git(
        None,
        state_dir,
        GitMode::Https,
        ["ls-remote", "--symref", upstream_url, "HEAD"].as_slice(),
    )?;
    Ok(output.lines().find_map(parse_symref_line))
}

fn resolve_https_checkout_ref(
    upstream_url: &str,
    default_ref: Option<&str>,
    override_ref: Option<&str>,
    state_dir: &Path,
) -> Result<Option<String>, WorkspaceCheckoutError> {
    match override_ref.or(default_ref) {
        Some(reference) => Ok(Some(reference.to_string())),
        None => resolve_remote_head_ref(upstream_url, state_dir),
    }
}

fn initialize_https_checkout(
    checkout_path: &Path,
    upstream_url: &str,
    state_dir: &Path,
) -> Result<(), WorkspaceCheckoutError> {
    let checkout_path_string = checkout_path.to_string_lossy().to_string();
    let init_args = ["init", checkout_path_string.as_str()];
    run_git(None, state_dir, GitMode::Https, &init_args)?;
    let remote_args = ["remote", "add", "origin", upstream_url];
    run_git(Some(checkout_path), state_dir, GitMode::Https, &remote_args)?;
    Ok(())
}

fn fetch_https_checkout(
    checkout_path: &Path,
    resolved_ref: Option<&str>,
    state_dir: &Path,
) -> Result<(), WorkspaceCheckoutError> {
    let fetch_target = resolved_ref.unwrap_or("HEAD");
    let fetch_args = ["fetch", "--depth", "1", "origin", fetch_target];
    run_git(Some(checkout_path), state_dir, GitMode::Https, &fetch_args)?;
    Ok(())
}

fn checkout_fetch_head(
    checkout_path: &Path,
    state_dir: &Path,
    mode: GitMode,
) -> Result<(), WorkspaceCheckoutError> {
    let checkout_args = ["checkout", "--detach", "FETCH_HEAD"];
    run_git(Some(checkout_path), state_dir, mode, &checkout_args)?;
    Ok(())
}

fn parse_symref_line(line: &str) -> Option<String> {
    let line = line.strip_prefix("ref: ")?;
    let (reference, target) = line.split_once('\t')?;
    (target == "HEAD").then(|| reference.to_string())
}

fn git_symbolic_ref(
    cwd: &Path,
    state_dir: &Path,
    mode: GitMode,
) -> Result<Option<String>, WorkspaceCheckoutError> {
    match run_git(
        Some(cwd),
        state_dir,
        mode,
        ["symbolic-ref", "-q", "HEAD"].as_slice(),
    ) {
        Ok(output) => Ok(Some(output.trim().to_string())),
        Err(WorkspaceCheckoutError::Git(_)) => Ok(None),
        Err(error) => Err(error),
    }
}

fn local_source_root(state_dir: &Path) -> Result<PathBuf, WorkspaceCheckoutError> {
    let current_dir = env::current_dir().map_err(|error| {
        WorkspaceCheckoutError::Io(format!(
            "reading the current working directory failed: {error}"
        ))
    })?;
    local_source_root_from(&current_dir, state_dir)
}

fn local_source_root_from(
    current_dir: &Path,
    state_dir: &Path,
) -> Result<PathBuf, WorkspaceCheckoutError> {
    let source_root = run_git(
        Some(current_dir),
        state_dir,
        GitMode::Local,
        &["rev-parse", "--show-toplevel"],
    )?;
    Ok(PathBuf::from(source_root.trim()))
}

fn resolve_local_checkout_ref(
    source_root: &Path,
    default_ref: Option<&str>,
    override_ref: Option<&str>,
    state_dir: &Path,
) -> Result<Option<String>, WorkspaceCheckoutError> {
    match override_ref.or(default_ref) {
        Some(reference) => Ok(Some(reference.to_string())),
        None => git_symbolic_ref(source_root, state_dir, GitMode::Local),
    }
}

fn clone_local_repository(
    source_root: &Path,
    checkout_path: &Path,
    state_dir: &Path,
) -> Result<(), WorkspaceCheckoutError> {
    let source_root_string = source_root.to_string_lossy().to_string();
    let checkout_path_string = checkout_path.to_string_lossy().to_string();
    let clone_args = [
        "clone",
        "--no-local",
        source_root_string.as_str(),
        checkout_path_string.as_str(),
    ];
    run_git(None, state_dir, GitMode::Local, &clone_args)?;
    Ok(())
}

fn checkout_local_ref_if_needed(
    checkout_path: &Path,
    resolved_ref: Option<&str>,
    state_dir: &Path,
) -> Result<(), WorkspaceCheckoutError> {
    let Some(reference) = resolved_ref else {
        return Ok(());
    };
    let checkout_args = ["checkout", "--detach", reference];
    run_git(
        Some(checkout_path),
        state_dir,
        GitMode::Local,
        &checkout_args,
    )?;
    Ok(())
}

fn checkout_head_commit(
    checkout_path: &Path,
    state_dir: &Path,
    mode: GitMode,
) -> Result<Option<String>, WorkspaceCheckoutError> {
    let head = run_git(Some(checkout_path), state_dir, mode, &["rev-parse", "HEAD"])?;
    Ok(Some(head.trim().to_string()))
}

fn build_prepared_checkout(
    checkout_relpath: String,
    checkout_ref: Option<String>,
    checkout_commit_sha: Option<String>,
    checkout_path: &Path,
) -> PreparedWorkspaceCheckout {
    PreparedWorkspaceCheckout {
        checkout_relpath,
        checkout_ref,
        checkout_commit_sha,
        working_dir: checkout_path.to_path_buf(),
    }
}

fn run_git(
    cwd: Option<&Path>,
    state_dir: &Path,
    mode: GitMode,
    args: &[&str],
) -> Result<String, WorkspaceCheckoutError> {
    let git_home = state_dir.join("git-home");
    fs::create_dir_all(&git_home).map_err(|error| {
        WorkspaceCheckoutError::Io(format!("creating git home failed: {error}"))
    })?;

    let mut command = git_command();
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    command.args(safe_git_config_args(mode));
    command.args(args);
    command.env_clear();
    if let Some(path) = env::var_os("PATH") {
        command.env("PATH", path);
    }
    if let Some(lang) = env::var_os("LANG") {
        command.env("LANG", lang);
    }
    if let Some(tmpdir) = env::var_os("TMPDIR") {
        command.env("TMPDIR", tmpdir);
    }
    command.env("HOME", git_home);
    command.env("GIT_TERMINAL_PROMPT", "0");
    command.env("GIT_CONFIG_NOSYSTEM", "1");
    command.env("GIT_CONFIG_GLOBAL", "/dev/null");

    let output = command
        .output()
        .map_err(|error| WorkspaceCheckoutError::Io(format!("running git failed: {error}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let detail = if stderr.is_empty() {
            format!("git exited with status {}", output.status)
        } else {
            stderr
        };
        return Err(WorkspaceCheckoutError::Git(detail));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[cfg(test)]
fn git_command() -> Command {
    TEST_GIT_BIN_OVERRIDE.with(|override_path| {
        if let Some(override_path) = override_path.borrow().as_ref().cloned() {
            let mut command = Command::new(override_path.program);
            command.args(override_path.prefix_args);
            command
        } else {
            Command::new("git")
        }
    })
}

#[cfg(not(test))]
fn git_command() -> Command {
    Command::new("git")
}

fn safe_git_config_args(mode: GitMode) -> Vec<String> {
    let mut configs = vec![
        ("core.fsmonitor", "false"),
        ("core.attributesFile", "/dev/null"),
        ("credential.helper", ""),
        ("core.sshCommand", ""),
        ("protocol.allow", "never"),
        ("protocol.https.allow", "always"),
        ("protocol.git.allow", "never"),
        ("protocol.ssh.allow", "never"),
        ("protocol.ext.allow", "never"),
        ("http.followRedirects", "false"),
        ("transfer.bundleURI", "false"),
        ("submodule.recurse", "false"),
        ("commit.gpgSign", "false"),
        ("tag.gpgSign", "false"),
        ("diff.submodule", "false"),
        ("status.submoduleSummary", "false"),
    ];
    if should_disable_repo_hooks(
        env::var_os("CONTROL_PLANE_FAST_EXECUTION_GIT_HOOKS_SOURCE").as_deref(),
    ) {
        configs.push(("core.hooksPath", "/dev/null"));
    }
    if mode == GitMode::Local {
        configs.push(("protocol.file.allow", "always"));
    } else {
        configs.push(("protocol.file.allow", "never"));
    }

    configs
        .into_iter()
        .flat_map(|(key, value)| ["-c".to_string(), format!("{key}={value}")])
        .collect()
}

fn should_disable_repo_hooks(control_plane_hooks_source: Option<&OsStr>) -> bool {
    control_plane_hooks_source.is_none()
}


#[cfg(test)]
mod tests {
    use super::*;
    mod checkout;
}
