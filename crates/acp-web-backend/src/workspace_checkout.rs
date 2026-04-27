use std::{
    collections::BTreeMap,
    env,
    ffi::OsStr,
    fs,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use async_trait::async_trait;
use git2::{
    BranchType, Direction, ErrorCode, FetchOptions, ProxyOptions, RemoteCallbacks, RemoteRedirect,
    Repository, RepositoryInitOptions, build::CheckoutBuilder,
};
use reqwest::Url;

use crate::{contract_workspaces::WorkspaceBranch, workspace_records::WorkspaceRecord};

const CHECKOUTS_DIR_NAME: &str = "session-checkouts";
const GIT_FETCH_HEAD: &str = "FETCH_HEAD";
const GIT_HOME_DIR_NAME: &str = "git-home";
const GIT_REMOTE_NAME: &str = "origin";

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

    async fn list_branches(
        &self,
        _workspace: &WorkspaceRecord,
    ) -> Result<Vec<WorkspaceBranch>, WorkspaceCheckoutError> {
        Ok(Vec::new())
    }

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
            ),
            None => self.clone_local_workspace(
                workspace.default_ref.as_deref(),
                validated_override.as_deref(),
                &checkout_path,
            ),
        }
        .map(|(resolved_ref, checkout_commit_sha)| {
            build_prepared_checkout(
                checkout_relpath,
                resolved_ref,
                checkout_commit_sha,
                &checkout_path,
            )
        });

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
    ) -> Result<(Option<String>, Option<String>), WorkspaceCheckoutError> {
        validate_https_upstream_url(upstream_url)?;
        let resolved_ref =
            resolve_https_checkout_ref(upstream_url, default_ref, override_ref, &self.state_dir)?;
        clone_remote_workspace(
            upstream_url,
            resolved_ref.as_deref(),
            checkout_path,
            &self.state_dir,
        )
        .map(|checkout_commit_sha| (resolved_ref, checkout_commit_sha))
    }

    fn clone_local_workspace(
        &self,
        default_ref: Option<&str>,
        override_ref: Option<&str>,
        checkout_path: &Path,
    ) -> Result<(Option<String>, Option<String>), WorkspaceCheckoutError> {
        let source_root = local_source_root(&self.state_dir)?;
        let resolved_ref =
            resolve_local_checkout_ref(&source_root, default_ref, override_ref, &self.state_dir)?;
        clone_local_repository(
            &source_root,
            resolved_ref.as_deref(),
            checkout_path,
            &self.state_dir,
        )
        .map(|checkout_commit_sha| (resolved_ref, checkout_commit_sha))
    }

    fn list_branches_sync(
        &self,
        workspace: &WorkspaceRecord,
    ) -> Result<Vec<WorkspaceBranch>, WorkspaceCheckoutError> {
        match workspace.upstream_url.as_deref() {
            Some(upstream_url) => list_remote_workspace_branches(upstream_url, &self.state_dir),
            None => {
                let source_root = local_source_root(&self.state_dir)?;
                list_local_workspace_branches(&source_root, &self.state_dir)
            }
        }
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

    async fn list_branches(
        &self,
        workspace: &WorkspaceRecord,
    ) -> Result<Vec<WorkspaceBranch>, WorkspaceCheckoutError> {
        let manager = self.clone();
        let workspace = workspace.clone();

        await_branch_task(tokio::task::spawn_blocking(move || {
            manager.list_branches_sync(&workspace)
        }))
        .await
    }
}

async fn await_checkout_task(
    handle: tokio::task::JoinHandle<Result<PreparedWorkspaceCheckout, WorkspaceCheckoutError>>,
) -> Result<PreparedWorkspaceCheckout, WorkspaceCheckoutError> {
    handle.await.map_err(|error| {
        WorkspaceCheckoutError::Io(format!("joining checkout task failed: {error}"))
    })?
}

async fn await_branch_task(
    handle: tokio::task::JoinHandle<Result<Vec<WorkspaceBranch>, WorkspaceCheckoutError>>,
) -> Result<Vec<WorkspaceBranch>, WorkspaceCheckoutError> {
    handle.await.map_err(|error| {
        WorkspaceCheckoutError::Io(format!("joining branch lookup task failed: {error}"))
    })?
}

fn checkout_parent_dir(checkout_path: &Path) -> Result<&Path, WorkspaceCheckoutError> {
    checkout_path.parent().ok_or_else(|| {
        WorkspaceCheckoutError::Io("session checkout path must have a parent directory".to_string())
    })
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
    with_probe_repository(state_dir, "remote-head", |repo| {
        let mut remote = repo.remote_anonymous(upstream_url).map_err(map_git_error)?;
        let connection = remote
            .connect_auth(
                Direction::Fetch,
                Some(git_remote_callbacks()),
                Some(git_proxy_options()),
            )
            .map_err(map_git_error)?;
        parse_remote_default_branch(connection.default_branch().map_err(map_git_error)?)
    })
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

fn list_remote_workspace_branches(
    upstream_url: &str,
    state_dir: &Path,
) -> Result<Vec<WorkspaceBranch>, WorkspaceCheckoutError> {
    validate_branch_list_upstream_url(upstream_url)?;
    with_probe_repository(state_dir, "remote-branches", |repo| {
        let mut remote = repo.remote_anonymous(upstream_url).map_err(map_git_error)?;
        let connection = remote
            .connect_auth(
                Direction::Fetch,
                Some(git_remote_callbacks()),
                Some(git_proxy_options()),
            )
            .map_err(map_git_error)?;
        let default_ref =
            parse_remote_default_branch(connection.default_branch().map_err(map_git_error)?)?;
        let mut branches = workspace_branches_from_refs(
            connection
                .list()
                .map_err(map_git_error)?
                .iter()
                .map(|head| head.name()),
        );
        prioritize_workspace_branch_ref(&mut branches, default_ref.as_deref());
        Ok(branches)
    })
}

fn validate_branch_list_upstream_url(upstream_url: &str) -> Result<(), WorkspaceCheckoutError> {
    if cfg!(test) && upstream_url.starts_with("file://") {
        return Ok(());
    }
    validate_https_upstream_url(upstream_url)
}

fn clone_remote_workspace(
    upstream_url: &str,
    resolved_ref: Option<&str>,
    checkout_path: &Path,
    state_dir: &Path,
) -> Result<Option<String>, WorkspaceCheckoutError> {
    fetch_workspace_checkout(upstream_url, resolved_ref, checkout_path, state_dir)
}

fn fetch_workspace_checkout(
    remote_spec: &str,
    resolved_ref: Option<&str>,
    checkout_path: &Path,
    state_dir: &Path,
) -> Result<Option<String>, WorkspaceCheckoutError> {
    let _git_home = ensure_git_home(state_dir)?;
    let repo = init_repository(checkout_path)?;
    repo.remote(GIT_REMOTE_NAME, remote_spec)
        .map_err(map_git_error)?;
    {
        let mut remote = repo.find_remote(GIT_REMOTE_NAME).map_err(map_git_error)?;
        let mut fetch_options = git_fetch_options(remote_spec);
        remote
            .fetch(
                &[resolved_ref.unwrap_or("HEAD")],
                Some(&mut fetch_options),
                None,
            )
            .map_err(map_git_error)?;
    }
    checkout_fetch_head(checkout_path, state_dir)?;
    checkout_head_commit(checkout_path, state_dir)
}

fn checkout_fetch_head(
    checkout_path: &Path,
    state_dir: &Path,
) -> Result<(), WorkspaceCheckoutError> {
    let _git_home = ensure_git_home(state_dir)?;
    let repo = Repository::open(checkout_path).map_err(map_git_error)?;
    checkout_revision(&repo, GIT_FETCH_HEAD)
}

fn git_symbolic_ref(
    cwd: &Path,
    state_dir: &Path,
) -> Result<Option<String>, WorkspaceCheckoutError> {
    let _git_home = ensure_git_home(state_dir)?;
    let repo = Repository::discover(cwd).map_err(map_git_error)?;
    if repo.head_detached().map_err(map_git_error)? {
        return Ok(None);
    }
    match repo.head() {
        Ok(head) => Ok(head.name().map(str::to_string)),
        Err(error) if matches!(error.code(), ErrorCode::NotFound | ErrorCode::UnbornBranch) => {
            Ok(None)
        }
        Err(error) => Err(map_git_error(error)),
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
    let _git_home = ensure_git_home(state_dir)?;
    let repo = Repository::discover(current_dir).map_err(map_git_error)?;
    repo.workdir().map(Path::to_path_buf).ok_or_else(|| {
        WorkspaceCheckoutError::Git("repository root has no working directory".to_string())
    })
}

fn resolve_local_checkout_ref(
    source_root: &Path,
    default_ref: Option<&str>,
    override_ref: Option<&str>,
    state_dir: &Path,
) -> Result<Option<String>, WorkspaceCheckoutError> {
    match override_ref.or(default_ref) {
        Some(reference) => Ok(Some(reference.to_string())),
        None => git_symbolic_ref(source_root, state_dir),
    }
}

fn clone_local_repository(
    source_root: &Path,
    resolved_ref: Option<&str>,
    checkout_path: &Path,
    state_dir: &Path,
) -> Result<Option<String>, WorkspaceCheckoutError> {
    let source_root = source_root.to_string_lossy().to_string();
    fetch_workspace_checkout(&source_root, resolved_ref, checkout_path, state_dir)
}

fn list_local_workspace_branches(
    source_root: &Path,
    state_dir: &Path,
) -> Result<Vec<WorkspaceBranch>, WorkspaceCheckoutError> {
    let _git_home = ensure_git_home(state_dir)?;
    let repo = Repository::discover(source_root).map_err(map_git_error)?;
    let default_ref = git_symbolic_ref(source_root, state_dir)?;
    let branches = repo
        .branches(Some(BranchType::Local))
        .map_err(map_git_error)?;
    let mut ref_names = Vec::new();

    for branch in branches {
        let (branch, _) = branch.map_err(map_git_error)?;
        let reference = branch.into_reference();
        let Some(name) = reference.name() else {
            continue;
        };
        ref_names.push(name.to_string());
    }

    let mut branches = workspace_branches_from_refs(ref_names.iter().map(String::as_str));
    prioritize_workspace_branch_ref(&mut branches, default_ref.as_deref());
    Ok(branches)
}

fn checkout_head_commit(
    checkout_path: &Path,
    state_dir: &Path,
) -> Result<Option<String>, WorkspaceCheckoutError> {
    let _git_home = ensure_git_home(state_dir)?;
    let repo = Repository::open(checkout_path).map_err(map_git_error)?;
    current_head_commit(&repo)
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

fn with_probe_repository<T>(
    state_dir: &Path,
    prefix: &str,
    operation: impl FnOnce(&Repository) -> Result<T, WorkspaceCheckoutError>,
) -> Result<T, WorkspaceCheckoutError> {
    let git_home = ensure_git_home(state_dir)?;
    let probe_dir = git_home.join(format!("{prefix}-{}", uuid::Uuid::new_v4().simple()));
    fs::create_dir_all(&probe_dir).map_err(|error| {
        WorkspaceCheckoutError::Io(format!("creating git home failed: {error}"))
    })?;
    let repo = init_bare_repository(&probe_dir)?;
    let result = operation(&repo);
    let _ = fs::remove_dir_all(&probe_dir);
    result
}

fn ensure_git_home(state_dir: &Path) -> Result<PathBuf, WorkspaceCheckoutError> {
    let git_home = state_dir.join(GIT_HOME_DIR_NAME);
    fs::create_dir_all(&git_home).map_err(|error| {
        WorkspaceCheckoutError::Io(format!("creating git home failed: {error}"))
    })?;
    Ok(git_home)
}

fn init_repository(path: &Path) -> Result<Repository, WorkspaceCheckoutError> {
    let mut options = RepositoryInitOptions::new();
    options.external_template(false);
    options.no_reinit(true);
    Repository::init_opts(path, &options).map_err(map_git_error)
}

fn init_bare_repository(path: &Path) -> Result<Repository, WorkspaceCheckoutError> {
    let mut options = RepositoryInitOptions::new();
    options.bare(true);
    options.external_template(false);
    options.no_reinit(true);
    Repository::init_opts(path, &options).map_err(map_git_error)
}

fn git_fetch_options(remote_url: &str) -> FetchOptions<'static> {
    let mut options = FetchOptions::new();
    if supports_shallow_fetch(remote_url) {
        options.depth(1);
    }
    options.follow_redirects(RemoteRedirect::None);
    options.proxy_options(git_proxy_options());
    options.remote_callbacks(git_remote_callbacks());
    options
}

fn supports_shallow_fetch(remote_url: &str) -> bool {
    !remote_url.starts_with("file://") && !Path::new(remote_url).is_absolute()
}

fn git_remote_callbacks() -> RemoteCallbacks<'static> {
    let mut callbacks = RemoteCallbacks::new();
    callbacks.credentials(reject_git_credentials);
    callbacks
}

fn reject_git_credentials(
    _url: &str,
    _username_from_url: Option<&str>,
    _allowed: git2::CredentialType,
) -> Result<git2::Cred, git2::Error> {
    Err(git2::Error::from_str(
        "credentialed git transports are not supported",
    ))
}

fn git_proxy_options() -> ProxyOptions<'static> {
    ProxyOptions::new()
}

fn parse_remote_default_branch(
    default_branch: git2::Buf,
) -> Result<Option<String>, WorkspaceCheckoutError> {
    parse_remote_default_branch_name(default_branch.as_str())
}

fn parse_remote_default_branch_name(
    default_branch: Option<&str>,
) -> Result<Option<String>, WorkspaceCheckoutError> {
    match default_branch {
        Some(reference) => Ok(Some(reference.to_string())),
        None => Err(WorkspaceCheckoutError::Git(
            "remote default branch is not valid UTF-8".to_string(),
        )),
    }
}

fn workspace_branches_from_refs<'a>(
    refs: impl IntoIterator<Item = &'a str>,
) -> Vec<WorkspaceBranch> {
    let mut branches = BTreeMap::new();
    for reference in refs {
        let Some(name) = reference.strip_prefix("refs/heads/") else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        branches.insert(
            reference.to_string(),
            WorkspaceBranch {
                name: name.to_string(),
                ref_name: reference.to_string(),
            },
        );
    }
    branches.into_values().collect()
}

fn prioritize_workspace_branch_ref(
    branches: &mut Vec<WorkspaceBranch>,
    preferred_ref: Option<&str>,
) {
    let Some(preferred_ref) = preferred_ref else {
        return;
    };
    let Some(index) = branches
        .iter()
        .position(|branch| branch.ref_name == preferred_ref)
    else {
        return;
    };
    let preferred_branch = branches.remove(index);
    branches.insert(0, preferred_branch);
}

fn checkout_builder() -> CheckoutBuilder<'static> {
    let mut builder = CheckoutBuilder::new();
    builder.force();
    builder.disable_filters(true);
    builder
}

fn checkout_revision(repo: &Repository, spec: &str) -> Result<(), WorkspaceCheckoutError> {
    let object = repo.revparse_single(spec).map_err(map_git_error)?;
    let commit = object.peel_to_commit().map_err(map_git_error)?;
    let mut builder = checkout_builder();
    repo.checkout_tree(commit.as_object(), Some(&mut builder))
        .map_err(map_git_error)?;
    repo.set_head_detached(commit.id()).map_err(map_git_error)?;
    Ok(())
}

fn current_head_commit(repo: &Repository) -> Result<Option<String>, WorkspaceCheckoutError> {
    match repo.head() {
        Ok(head) => match head.peel_to_commit() {
            Ok(commit) => Ok(Some(commit.id().to_string())),
            Err(error) if matches!(error.code(), ErrorCode::NotFound | ErrorCode::UnbornBranch) => {
                Ok(None)
            }
            Err(error) => Err(map_git_error(error)),
        },
        Err(error) if matches!(error.code(), ErrorCode::NotFound | ErrorCode::UnbornBranch) => {
            Ok(None)
        }
        Err(error) => Err(map_git_error(error)),
    }
}

fn map_git_error(error: git2::Error) -> WorkspaceCheckoutError {
    let message = error.message().trim();
    if message.is_empty() {
        WorkspaceCheckoutError::Git(format!("git operation failed ({:?})", error.code()))
    } else {
        WorkspaceCheckoutError::Git(message.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    mod checkout;
}
