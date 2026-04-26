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
    use chrono::Utc;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;

    fn test_root_dir() -> PathBuf {
        let root = std::env::current_dir()
            .expect("workspace checkout tests should start in a readable directory")
            .join(".tmp");
        std::fs::create_dir_all(&root).expect("workspace checkout test root should be creatable");
        root
    }

    fn unique_test_dir(prefix: &str) -> PathBuf {
        test_root_dir().join(format!("{prefix}-{}", uuid::Uuid::new_v4().simple()))
    }

    const TEST_BRANCH: &str = "test-branch";

    fn sample_workspace_record(
        upstream_url: Option<&str>,
        default_ref: Option<&str>,
    ) -> WorkspaceRecord {
        WorkspaceRecord {
            workspace_id: "w_test".to_string(),
            owner_user_id: "u_test".to_string(),
            name: "Workspace".to_string(),
            upstream_url: upstream_url.map(str::to_string),
            default_ref: default_ref.map(str::to_string),
            credential_reference_id: None,
            bootstrap_kind: None,
            status: "active".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            deleted_at: None,
        }
    }

    fn run_plain_git(cwd: Option<&Path>, args: &[&str]) -> String {
        let mut command = Command::new("git");
        if let Some(cwd) = cwd {
            command.current_dir(cwd);
        }
        command.args(args);
        let output = command.output().expect("git command should start");
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        assert!(output.status.success(), "git {:?} failed: {stderr}", args);
        String::from_utf8_lossy(&output.stdout).to_string()
    }

    fn initialize_local_repo(path: &Path) -> String {
        let path_string = path.to_string_lossy().to_string();
        run_plain_git(
            None,
            [
                "init",
                "--initial-branch",
                TEST_BRANCH,
                path_string.as_str(),
            ]
            .as_slice(),
        );
        run_plain_git(Some(path), ["config", "user.name", "Test User"].as_slice());
        run_plain_git(
            Some(path),
            ["config", "user.email", "test@example.com"].as_slice(),
        );
        std::fs::write(path.join("fixture.txt"), "hello\n")
            .expect("test repository files should be writable");
        run_plain_git(Some(path), ["add", "fixture.txt"].as_slice());
        run_plain_git(Some(path), ["commit", "-m", "initial"].as_slice());
        run_plain_git(Some(path), ["tag", "v1"].as_slice());
        run_plain_git(Some(path), ["rev-parse", "HEAD"].as_slice())
            .trim()
            .to_string()
    }

    fn run_self_test_child(test_name: &str, extra_env: &[(&str, &str)]) -> std::process::Output {
        let mut command =
            Command::new(std::env::current_exe().expect("current test binary should exist"));
        command.arg("--exact").arg(test_name).arg("--nocapture");
        for (key, value) in extra_env {
            command.env(key, value);
        }
        command
            .output()
            .expect("child test process should be spawnable")
    }

    fn assert_child_success(output: std::process::Output) {
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(!stdout.contains("0 passed"));
    }

    #[cfg(unix)]
    const FAKE_GIT_SCRIPT: &str = r#"#!/usr/bin/env python3
import os
import pathlib
import sys

args = sys.argv[1:]
filtered = []
index = 0
while index < len(args):
    if args[index] == "-c":
        index += 2
        continue
    filtered.append(args[index])
    index += 1

command = filtered[0] if filtered else ""

if command == "ls-remote":
    print("ref: refs/heads/main\tHEAD")
elif command == "init":
    checkout = pathlib.Path(filtered[1])
    checkout.mkdir(parents=True, exist_ok=True)
    (checkout / ".git").mkdir(exist_ok=True)
elif command == "remote" and filtered[1:3] == ["add", "origin"]:
    pass
elif command == "fetch":
    if filtered[-1] == "refs/heads/missing":
        sys.stderr.write("fatal: missing ref\n")
        sys.exit(44)
elif command == "checkout" and filtered[1:3] == ["--detach", "FETCH_HEAD"]:
    pass
elif command == "rev-parse" and filtered[1] == "HEAD":
    print("deadbeef")
elif command == "symbolic-ref":
    print("refs/heads/main")
elif command == "print-tmpdir":
    print(os.environ.get("TMPDIR", ""))
elif command == "fail-empty-stderr":
    sys.exit(42)
elif command == "fail-with-stderr":
    sys.stderr.write("fatal: bad thing\n")
    sys.exit(43)
else:
    sys.stderr.write(f"unexpected fake git command: {filtered}\n")
    sys.exit(99)
"#;

    #[cfg(unix)]
    struct TestGitOverrideGuard(Option<super::TestGitCommand>);

    #[cfg(unix)]
    static TMPDIR_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[cfg(unix)]
    struct TmpdirEnvGuard(Option<std::ffi::OsString>);

    #[cfg(unix)]
    impl Drop for TmpdirEnvGuard {
        fn drop(&mut self) {
            match self.0.take() {
                Some(value) => unsafe {
                    // Tests serialize TMPDIR mutation with TMPDIR_ENV_LOCK.
                    std::env::set_var("TMPDIR", value)
                },
                None => unsafe {
                    // Tests serialize TMPDIR mutation with TMPDIR_ENV_LOCK.
                    std::env::remove_var("TMPDIR")
                },
            }
        }
    }

    #[cfg(unix)]
    impl Drop for TestGitOverrideGuard {
        fn drop(&mut self) {
            super::TEST_GIT_BIN_OVERRIDE.with(|override_path| {
                override_path.replace(self.0.take());
            });
        }
    }

    #[cfg(unix)]
    fn override_git_for_current_thread(path: &Path) -> TestGitOverrideGuard {
        let previous = super::TEST_GIT_BIN_OVERRIDE.with(|override_path| {
            override_path.replace(Some(super::TestGitCommand {
                program: PathBuf::from("python3"),
                prefix_args: vec![path.as_os_str().to_os_string()],
            }))
        });
        TestGitOverrideGuard(previous)
    }

    #[cfg(unix)]
    fn make_executable(script_path: &Path) {
        let mut permissions = std::fs::metadata(script_path)
            .expect("fake git script metadata should exist")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(script_path, permissions)
            .expect("fake git script should be executable");
    }

    #[cfg(unix)]
    fn write_fake_git_script(bin_dir: &Path) -> PathBuf {
        let script_path = bin_dir.join("git");
        std::fs::create_dir_all(bin_dir).expect("fake git bin dir should be creatable");
        std::fs::write(&script_path, FAKE_GIT_SCRIPT).expect("fake git script should be writable");
        make_executable(&script_path);
        script_path
    }

    fn assert_git_error_contains(error: WorkspaceCheckoutError, expected: &str) {
        assert!(matches!(error, WorkspaceCheckoutError::Git(_)), "{error:?}");
        assert!(error.message().contains(expected), "{}", error.message());
    }

    fn assert_io_error_contains(error: WorkspaceCheckoutError, expected: &str) {
        assert!(matches!(error, WorkspaceCheckoutError::Io(_)), "{error:?}");
        assert!(error.message().contains(expected), "{}", error.message());
    }

    #[test]
    fn checkout_ref_validation_accepts_safe_refs_and_rejects_unsafe_values() {
        assert_eq!(
            validate_checkout_ref(Some(" refs/heads/main ")).expect("ref should validate"),
            Some("refs/heads/main".to_string())
        );
        assert_eq!(
            validate_checkout_ref(Some("   ")).expect_err("blank refs should fail"),
            WorkspaceCheckoutError::Validation("checkout_ref must not be empty".to_string())
        );
        assert_eq!(
            validate_checkout_ref(Some("feature branch")).expect_err("whitespace should fail"),
            WorkspaceCheckoutError::Validation("checkout_ref is invalid".to_string())
        );
        assert_eq!(
            validate_checkout_ref(Some("-branch")).expect_err("dash-prefixed refs should fail"),
            WorkspaceCheckoutError::Validation("checkout_ref is invalid".to_string())
        );
    }

    #[test]
    fn symref_parsing_extracts_head_targets() {
        assert_eq!(
            parse_symref_line("ref: refs/heads/main\tHEAD"),
            Some("refs/heads/main".to_string())
        );
        assert_eq!(parse_symref_line("deadbeef\tHEAD"), None);
    }

    #[test]
    fn safe_git_configs_allow_file_only_for_local_mode() {
        let local_args = safe_git_config_args(GitMode::Local).join(" ");
        let https_args = safe_git_config_args(GitMode::Https).join(" ");

        assert!(local_args.contains("protocol.file.allow=always"));
        assert!(https_args.contains("protocol.file.allow=never"));
        assert!(https_args.contains("http.followRedirects=false"));
    }

    #[test]
    fn control_plane_managed_hooks_disable_core_hook_overrides() {
        assert!(should_disable_repo_hooks(None));
        assert!(!should_disable_repo_hooks(Some(OsStr::new(
            "/environment/hooks/git"
        ))));
    }

    #[test]
    fn workspace_checkout_errors_expose_messages_through_display() {
        let errors = [
            WorkspaceCheckoutError::Validation("validation failed".to_string()),
            WorkspaceCheckoutError::Io("io failed".to_string()),
            WorkspaceCheckoutError::Git("git failed".to_string()),
        ];

        for error in errors {
            assert_eq!(error.message(), error.to_string());
            assert!(std::error::Error::source(&error).is_none());
        }
    }

    #[tokio::test]
    async fn workspace_checkout_manager_defaults_to_unresolved_paths() {
        #[derive(Debug)]
        struct NoopCheckoutManager;

        #[async_trait]
        impl WorkspaceCheckoutManager for NoopCheckoutManager {
            async fn prepare_checkout(
                &self,
                _workspace: &WorkspaceRecord,
                _session_id: &str,
                _checkout_ref_override: Option<&str>,
            ) -> Result<PreparedWorkspaceCheckout, WorkspaceCheckoutError> {
                Err(WorkspaceCheckoutError::Io(
                    "checkout preparation is intentionally unused".to_string(),
                ))
            }
        }

        assert_eq!(
            NoopCheckoutManager.resolve_checkout_path("session-checkouts/s_test"),
            None
        );
        assert_eq!(
            NoopCheckoutManager
                .prepare_checkout(&sample_workspace_record(None, None), "s_test", None)
                .await
                .expect_err("noop managers should reject checkout preparation"),
            WorkspaceCheckoutError::Io("checkout preparation is intentionally unused".to_string())
        );
    }

    #[test]
    fn https_upstream_validation_rejects_non_https_and_embedded_credentials() {
        assert_eq!(
            validate_https_upstream_url("not a url").expect_err("invalid URLs should fail"),
            WorkspaceCheckoutError::Validation("upstream_url must be a valid URL".to_string())
        );
        assert_eq!(
            validate_https_upstream_url("http://example.com/repo.git")
                .expect_err("non-https URLs should fail"),
            WorkspaceCheckoutError::Validation("upstream_url must use https".to_string())
        );
        assert_eq!(
            validate_https_upstream_url("https://alice:secret@example.com/repo.git")
                .expect_err("embedded credentials should fail"),
            WorkspaceCheckoutError::Validation(
                "upstream_url must not embed credentials".to_string()
            )
        );
        validate_https_upstream_url("https://example.com/repo.git")
            .expect("plain https URLs should validate");
    }

    #[test]
    fn checkout_ref_resolution_prefers_override_then_default() {
        let state_dir = unique_test_dir("acp-workspace-checkout-ref-resolution");

        assert_eq!(
            resolve_https_checkout_ref(
                "https://example.com/repo.git",
                Some("refs/heads/main"),
                Some("refs/tags/v1"),
                &state_dir,
            )
            .expect("override should short-circuit"),
            Some("refs/tags/v1".to_string())
        );
        assert_eq!(
            resolve_https_checkout_ref(
                "https://example.com/repo.git",
                Some("refs/heads/main"),
                None,
                &state_dir,
            )
            .expect("default should short-circuit"),
            Some("refs/heads/main".to_string())
        );
        assert_eq!(
            resolve_local_checkout_ref(
                Path::new("/workspace"),
                Some("refs/heads/main"),
                Some("refs/tags/v1"),
                &state_dir,
            )
            .expect("override should short-circuit"),
            Some("refs/tags/v1".to_string())
        );
        assert_eq!(
            resolve_local_checkout_ref(
                Path::new("/workspace"),
                Some("refs/heads/main"),
                None,
                &state_dir
            )
            .expect("default should short-circuit"),
            Some("refs/heads/main".to_string())
        );
    }

    #[test]
    fn build_prepared_checkout_preserves_supplied_fields() {
        let checkout_path = Path::new("/workspace/session-checkouts/s_test");

        assert_eq!(
            build_prepared_checkout(
                "session-checkouts/s_test".to_string(),
                Some("refs/heads/main".to_string()),
                Some("deadbeef".to_string()),
                checkout_path,
            ),
            PreparedWorkspaceCheckout {
                checkout_relpath: "session-checkouts/s_test".to_string(),
                checkout_ref: Some("refs/heads/main".to_string()),
                checkout_commit_sha: Some("deadbeef".to_string()),
                working_dir: checkout_path.to_path_buf(),
            }
        );
    }

    #[test]
    fn resolved_checkout_paths_stay_within_the_checkout_root() {
        let state_dir = unique_test_dir("acp-workspace-checkout-resolve");
        let manager = FsWorkspaceCheckoutManager::new(state_dir.clone());

        assert_eq!(
            manager.resolve_checkout_path("session-checkouts/s_test"),
            Some(state_dir.join("session-checkouts/s_test"))
        );
        assert_eq!(
            manager.resolve_checkout_path("session-checkouts/../escape"),
            None
        );
        assert_eq!(manager.resolve_checkout_path("../escape"), None);
        assert_eq!(manager.resolve_checkout_path("/tmp/escape"), None);
        assert_eq!(manager.resolve_checkout_path("other-root/s_test"), None);
    }

    #[test]
    fn prepare_checkout_sync_reports_checkout_root_creation_failures() {
        let state_dir = unique_test_dir("acp-workspace-checkout-root-error");
        std::fs::create_dir_all(&state_dir).expect("state dir should be creatable");
        std::fs::write(state_dir.join(CHECKOUTS_DIR_NAME), "blocker")
            .expect("blocking file should be writable");
        let manager = FsWorkspaceCheckoutManager::new(state_dir);

        let error = manager
            .prepare_checkout_sync(&sample_workspace_record(None, None), "s_test", None)
            .expect_err("blocking files should make the checkout root fail");

        assert_io_error_contains(error, "creating checkout root failed");
    }

    #[test]
    fn checkout_parent_dir_rejects_parentless_paths() {
        let error = checkout_parent_dir(Path::new(""))
            .expect_err("empty paths should not resolve checkout parents");

        assert_eq!(
            error,
            WorkspaceCheckoutError::Io(
                "session checkout path must have a parent directory".to_string()
            )
        );
    }

    #[test]
    fn prepare_checkout_sync_reports_stale_checkout_cleanup_failures() {
        let state_dir = unique_test_dir("acp-workspace-checkout-stale-error");
        let checkout_path = state_dir.join("session-checkouts/s_test");
        std::fs::create_dir_all(
            checkout_path
                .parent()
                .expect("checkout path should have a parent"),
        )
        .expect("checkout parent should be creatable");
        std::fs::write(&checkout_path, "stale file")
            .expect("stale checkout marker should be writable");
        let manager = FsWorkspaceCheckoutManager::new(state_dir);

        let error = manager
            .prepare_checkout_sync(&sample_workspace_record(None, None), "s_test", None)
            .expect_err("file-based stale paths should fail cleanup");

        assert_io_error_contains(error, "clearing stale checkout directory failed");
    }

    #[test]
    fn https_checkout_failures_clean_up_partially_initialized_directories() {
        let state_dir = unique_test_dir("acp-workspace-checkout-https-failure");
        let manager = FsWorkspaceCheckoutManager::new(state_dir.clone());
        let workspace = sample_workspace_record(Some("https://127.0.0.1:9/repo.git"), None);

        let error = manager
            .prepare_checkout_sync(&workspace, "s_test", None)
            .expect_err("unreachable https remotes should fail");

        assert!(matches!(error, WorkspaceCheckoutError::Git(_)));
        assert!(
            !state_dir.join("session-checkouts/s_test").exists(),
            "failed https preparations should remove partial checkouts"
        );
    }

    #[test]
    fn local_checkout_helpers_cover_clone_checkout_and_commit_resolution() {
        let source_root = unique_test_dir("acp-workspace-checkout-source");
        let expected_head = initialize_local_repo(&source_root);
        let state_dir = unique_test_dir("acp-workspace-checkout-state");
        let checkout_path = unique_test_dir("acp-workspace-checkout-clone");

        clone_local_repository(&source_root, &checkout_path, &state_dir)
            .expect("local repositories should clone");
        checkout_local_ref_if_needed(&checkout_path, Some("v1"), &state_dir)
            .expect("named refs should be check-outable");
        assert_eq!(
            checkout_head_commit(&checkout_path, &state_dir, GitMode::Local)
                .expect("head commits should resolve"),
            Some(expected_head.clone())
        );

        let source_root_string = source_root.to_string_lossy().to_string();
        run_git(
            Some(&checkout_path),
            &state_dir,
            GitMode::Local,
            ["fetch", "--depth", "1", source_root_string.as_str(), "HEAD"].as_slice(),
        )
        .expect("fetching a local FETCH_HEAD should succeed");
        checkout_fetch_head(&checkout_path, &state_dir, GitMode::Local)
            .expect("FETCH_HEAD should be check-outable");
        assert_eq!(
            checkout_head_commit(&checkout_path, &state_dir, GitMode::Local)
                .expect("detached commits should resolve"),
            Some(expected_head)
        );
    }

    #[test]
    fn git_symbolic_ref_handles_detached_heads_and_io_failures() {
        let repo = unique_test_dir("acp-workspace-checkout-symbolic-ref");
        initialize_local_repo(&repo);
        let state_dir = unique_test_dir("acp-workspace-checkout-symbolic-state");

        assert_eq!(
            git_symbolic_ref(&repo, &state_dir, GitMode::Local)
                .expect("branch heads should resolve"),
            Some(format!("refs/heads/{TEST_BRANCH}"))
        );

        run_plain_git(Some(&repo), ["checkout", "--detach"].as_slice());
        assert_eq!(
            git_symbolic_ref(&repo, &state_dir, GitMode::Local)
                .expect("detached heads should not error"),
            None
        );

        let broken_state_dir = unique_test_dir("acp-workspace-checkout-symbolic-state-broken");
        std::fs::write(&broken_state_dir, "state file")
            .expect("state dir blocker should be writable");
        let error = git_symbolic_ref(&repo, &broken_state_dir, GitMode::Local)
            .expect_err("broken state dirs should surface io failures");
        assert!(
            matches!(error, WorkspaceCheckoutError::Io(message) if message.contains("creating git home failed"))
        );
    }

    #[test]
    fn run_git_reports_state_dir_creation_failures() {
        let broken_state_dir = unique_test_dir("acp-workspace-checkout-run-git-io");
        std::fs::create_dir_all(
            broken_state_dir
                .parent()
                .expect("state dir should have a parent"),
        )
        .expect("test parent should be creatable");
        std::fs::write(&broken_state_dir, "state file")
            .expect("state dir blocker should be writable");

        let error = run_git(
            None,
            &broken_state_dir,
            GitMode::Https,
            ["status"].as_slice(),
        )
        .expect_err("file-backed state dirs should fail");

        assert!(
            matches!(error, WorkspaceCheckoutError::Io(message) if message.contains("creating git home failed"))
        );
    }

    #[cfg(unix)]
    fn assert_fake_https_checkout_success(manager: &FsWorkspaceCheckoutManager, state_dir: &Path) {
        let checkout_path = state_dir.join("checkout");
        assert_eq!(
            resolve_remote_head_ref("https://example.test/repo.git", state_dir)
                .expect("fake git should resolve remote HEAD"),
            Some("refs/heads/main".to_string())
        );
        let checkout = manager
            .clone_https_workspace(
                "https://example.test/repo.git",
                None,
                None,
                &checkout_path,
                "session-checkouts/s_test".to_string(),
            )
            .expect("fake https checkouts should succeed");
        assert_eq!(checkout.checkout_ref, Some("refs/heads/main".to_string()));
        assert_eq!(checkout.checkout_commit_sha, Some("deadbeef".to_string()));
        assert_eq!(checkout.working_dir, checkout_path);
    }

    #[cfg(unix)]
    fn assert_fake_https_checkout_failure_cleans_up(manager: &FsWorkspaceCheckoutManager) {
        let error = manager
            .prepare_checkout_sync(
                &sample_workspace_record(
                    Some("https://example.test/repo.git"),
                    Some("refs/heads/missing"),
                ),
                "s_missing",
                None,
            )
            .expect_err("missing refs should fail fake https checkout preparation");
        assert_git_error_contains(error, "fatal: missing ref");
        assert!(
            !manager.checkout_path("s_missing").exists(),
            "failed preparations should remove the checkout directory"
        );
    }

    #[cfg(unix)]
    fn assert_fake_git_error_paths(state_dir: &Path) {
        assert_eq!(
            resolve_https_checkout_ref("https://example.test/repo.git", None, None, state_dir)
                .expect("HEAD discovery should resolve"),
            Some("refs/heads/main".to_string())
        );
        assert_git_error_contains(
            run_git(None, state_dir, GitMode::Https, &["fail-empty-stderr"])
                .expect_err("empty-stderr failures should still report exit status"),
            "git exited with status",
        );
        assert_git_error_contains(
            run_git(None, state_dir, GitMode::Https, &["fail-with-stderr"])
                .expect_err("stderr failures should preserve git details"),
            "fatal: bad thing",
        );
    }

    #[cfg(unix)]
    fn assert_tmpdir_is_forwarded(state_dir: &Path) {
        let tmpdir = unique_test_dir("acp-workspace-checkout-tmpdir");
        std::fs::create_dir_all(&tmpdir).expect("TMPDIR fixture should be creatable");
        let expected = tmpdir.to_string_lossy().to_string();
        let _guard = TMPDIR_ENV_LOCK
            .lock()
            .expect("TMPDIR lock should be acquirable");
        let _tmpdir_guard = TmpdirEnvGuard(std::env::var_os("TMPDIR"));
        unsafe {
            // Tests serialize TMPDIR mutation with TMPDIR_ENV_LOCK.
            std::env::set_var("TMPDIR", &tmpdir);
        }
        let output = run_git(None, state_dir, GitMode::Https, &["print-tmpdir"])
            .expect("fake git should expose TMPDIR");
        assert_eq!(output.trim(), expected);
    }

    #[cfg(unix)]
    #[test]
    fn fake_git_https_helpers_cover_success_and_empty_stderr_paths() {
        let fixture_dir = unique_test_dir("acp-workspace-checkout-fake-git");
        let script_path = write_fake_git_script(&fixture_dir.join("bin"));
        let _git_override = override_git_for_current_thread(&script_path);
        let state_dir = fixture_dir.join("state");
        let manager = FsWorkspaceCheckoutManager::new(state_dir.clone());

        assert_fake_https_checkout_success(&manager, &state_dir);
        assert_fake_git_error_paths(&state_dir);
        assert_tmpdir_is_forwarded(&state_dir);
        assert_fake_https_checkout_failure_cleans_up(&manager);
    }

    #[test]
    fn local_source_root_from_resolves_repository_roots() {
        let repo = unique_test_dir("acp-workspace-checkout-local-root");
        initialize_local_repo(&repo);
        let nested = repo.join("nested");
        std::fs::create_dir_all(&nested).expect("nested repository paths should be creatable");

        assert_eq!(
            local_source_root_from(
                &nested,
                &unique_test_dir("acp-workspace-checkout-local-state")
            )
            .expect("repository roots should resolve"),
            repo
        );
    }

    #[test]
    fn local_source_root_reports_deleted_current_directories() {
        const CHILD_ENV: &str = "ACP_WORKSPACE_CHECKOUT_DELETED_CWD_CHILD";
        if std::env::var_os(CHILD_ENV).is_some() {
            let deleted_dir = PathBuf::from(
                std::env::var("ACP_DELETED_CWD").expect("deleted cwd env should exist"),
            );
            std::fs::create_dir_all(&deleted_dir)
                .expect("deleted cwd should be creatable before removal");
            std::env::set_current_dir(&deleted_dir)
                .expect("child should be able to chdir into the test directory");
            std::fs::remove_dir_all(&deleted_dir)
                .expect("test directory should be removable after chdir");
            let state_dir = deleted_dir
                .parent()
                .expect("deleted cwd should have a parent")
                .join(format!(
                    "acp-workspace-checkout-cwd-state-{}",
                    uuid::Uuid::new_v4().simple()
                ));

            let error =
                local_source_root(&state_dir).expect_err("deleted working directories should fail");
            assert!(
                matches!(error, WorkspaceCheckoutError::Io(message) if message.contains("reading the current working directory failed"))
            );
            return;
        }

        let deleted_dir = unique_test_dir("acp-workspace-checkout-deleted-cwd");
        let deleted_dir_string = deleted_dir.to_string_lossy().to_string();
        let output = run_self_test_child(
            "workspace_checkout::tests::local_source_root_reports_deleted_current_directories",
            &[
                (CHILD_ENV, "1"),
                ("ACP_DELETED_CWD", deleted_dir_string.as_str()),
            ],
        );
        assert_child_success(output);
    }

    #[tokio::test]
    async fn await_checkout_task_maps_join_failures_into_io_errors() {
        let error = await_checkout_task(tokio::task::spawn_blocking(
            || -> Result<PreparedWorkspaceCheckout, WorkspaceCheckoutError> {
                panic!("simulated checkout panic");
            },
        ))
        .await
        .expect_err("panicking blocking tasks should surface join failures");

        assert!(
            matches!(error, WorkspaceCheckoutError::Io(message) if message.contains("joining checkout task failed"))
        );
    }

    #[tokio::test]
    async fn local_workspace_fallback_prepares_a_checkout_from_the_current_repo() {
        let state_dir = unique_test_dir("acp-workspace-checkout-test");
        let manager = FsWorkspaceCheckoutManager::new(state_dir);
        let workspace = sample_workspace_record(None, None);

        let checkout = manager
            .prepare_checkout(&workspace, "s_test", None)
            .await
            .expect("local checkout should prepare");

        assert_eq!(checkout.checkout_relpath, "session-checkouts/s_test");
        assert!(checkout.checkout_commit_sha.is_some());
        assert!(checkout.working_dir.join("Cargo.toml").exists());
    }
}
