use super::super::{
    CHECKOUTS_DIR_NAME, FsWorkspaceCheckoutManager, GIT_REMOTE_NAME, PreparedWorkspaceCheckout,
    WorkspaceCheckoutError, WorkspaceCheckoutManager, await_checkout_task, build_prepared_checkout,
    checkout_fetch_head, checkout_head_commit, checkout_parent_dir, clone_local_repository,
    clone_remote_workspace, current_head_commit, git_fetch_options, git_symbolic_ref,
    list_local_workspace_branches, local_source_root, local_source_root_from, map_git_error,
    parse_remote_default_branch_name, prioritize_workspace_branch_ref, reject_git_credentials,
    resolve_https_checkout_ref, resolve_local_checkout_ref, resolve_remote_head_ref,
    validate_checkout_ref, validate_https_upstream_url, workspace_branches_from_refs,
};
use super::*;
use async_trait::async_trait;
use chrono::Utc;
use git2::{
    CredentialType, ErrorClass, ErrorCode, Oid, Repository, RepositoryInitOptions, Signature,
    build::RepoBuilder,
};
use reqwest::Url;
use std::{
    path::{Path, PathBuf},
    process::Command,
};

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

fn test_signature() -> Signature<'static> {
    Signature::now("Test User", "test@example.com")
        .expect("test signatures should be constructible")
}

fn initialize_local_repo(path: &Path) -> String {
    let mut options = RepositoryInitOptions::new();
    options.external_template(false);
    options.initial_head(TEST_BRANCH);
    let repo = Repository::init_opts(path, &options).expect("test repositories should initialize");

    std::fs::write(path.join("fixture.txt"), "hello\n")
        .expect("test repository files should be writable");
    let mut index = repo.index().expect("repo index should be readable");
    index
        .add_path(Path::new("fixture.txt"))
        .expect("fixture file should be addable");
    let tree_id = index.write_tree().expect("tree should be writable");
    let tree = repo.find_tree(tree_id).expect("tree should be readable");
    let signature = test_signature();
    let commit_id = repo
        .commit(Some("HEAD"), &signature, &signature, "initial", &tree, &[])
        .expect("initial commit should succeed");
    let commit = repo
        .find_commit(commit_id)
        .expect("initial commit should be readable");
    repo.tag_lightweight("v1", commit.as_object(), false)
        .expect("lightweight tags should be creatable");
    commit_id.to_string()
}

fn initialize_unborn_repo(path: &Path) {
    let mut options = RepositoryInitOptions::new();
    options.external_template(false);
    options.initial_head(TEST_BRANCH);
    Repository::init_opts(path, &options).expect("empty repos should initialize");
}

fn create_bare_remote_repo(prefix: &str) -> (String, String) {
    let source_root = unique_test_dir(&format!("{prefix}-source"));
    let expected_head = initialize_local_repo(&source_root);
    let bare_dir = unique_test_dir(&format!("{prefix}-bare"));
    let mut builder = RepoBuilder::new();
    builder.bare(true);
    builder
        .clone(source_root.to_string_lossy().as_ref(), &bare_dir)
        .expect("bare remotes should clone");
    let url = Url::from_file_path(&bare_dir)
        .expect("bare repo paths should convert to file URLs")
        .to_string();
    (url, expected_head)
}

fn detach_head(repo_path: &Path) {
    let repo = Repository::open(repo_path).expect("repo should open");
    let commit = repo
        .head()
        .expect("repo should have HEAD")
        .peel_to_commit()
        .expect("HEAD should resolve to a commit");
    repo.set_head_detached(commit.id())
        .expect("detaching HEAD should succeed");
}

fn initialize_detached_head_only_repo(path: &Path) -> String {
    initialize_local_repo(path);
    let repo = Repository::open(path).expect("repo should open");
    let base_commit = repo
        .head()
        .expect("repo should have HEAD")
        .peel_to_commit()
        .expect("HEAD should resolve to a commit");
    repo.set_head_detached(base_commit.id())
        .expect("detaching HEAD should succeed");
    std::fs::write(path.join("detached.txt"), "detached\n")
        .expect("detached-head fixture files should be writable");
    let mut index = repo.index().expect("repo index should be readable");
    index
        .add_path(Path::new("detached.txt"))
        .expect("detached fixture file should be addable");
    let tree_id = index.write_tree().expect("tree should be writable");
    let tree = repo.find_tree(tree_id).expect("tree should be readable");
    let signature = test_signature();
    repo.commit(
        Some("HEAD"),
        &signature,
        &signature,
        "detached",
        &tree,
        &[&base_commit],
    )
    .expect("detached HEAD commits should succeed")
    .to_string()
}

fn corrupt_head_file(repo_path: &Path, contents: &[u8]) {
    let git_dir = Repository::open(repo_path)
        .expect("repo should open")
        .path()
        .to_path_buf();
    std::fs::write(git_dir.join("HEAD"), contents).expect("HEAD should be writable");
}

fn state_dir_with_probe_path_overflow(prefix: &str) -> PathBuf {
    let mut state_dir = unique_test_dir(prefix);
    std::fs::create_dir_all(&state_dir).expect("long state dir root should be creatable");
    let segment = "aaaaaaaaaaaaaaaa";
    let probe_suffix = format!("remote-head-{}", "a".repeat(32));
    while state_dir
        .join("git-home")
        .join(&probe_suffix)
        .to_string_lossy()
        .len()
        <= 4095
    {
        state_dir = state_dir.join(segment);
        std::fs::create_dir_all(&state_dir).expect("long state dirs should be creatable");
    }
    assert!(
        state_dir.join("git-home").to_string_lossy().len() < 4096,
        "git home path should remain short enough to create"
    );
    assert!(
        state_dir
            .join("git-home")
            .join(&probe_suffix)
            .to_string_lossy()
            .len()
            > 4095,
        "probe path should overflow the platform path limit"
    );
    state_dir
}

fn fetch_checkout_origin_head(checkout_path: &Path) {
    let repo = Repository::open(checkout_path).expect("checkout repo should open");
    let mut remote = repo
        .find_remote(GIT_REMOTE_NAME)
        .expect("clone should persist origin");
    let remote_url = remote.url().expect("origin should expose a URL");
    let mut fetch_options = git_fetch_options(remote_url);
    remote
        .fetch(&["HEAD"], Some(&mut fetch_options), None)
        .expect("origin HEAD should fetch");
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

fn assert_git_error(error: WorkspaceCheckoutError) {
    assert!(matches!(error, WorkspaceCheckoutError::Git(_)), "{error:?}");
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
        WorkspaceCheckoutError::Validation("upstream_url must not embed credentials".to_string())
    );
    validate_https_upstream_url("https://example.com/repo.git")
        .expect("plain https URLs should validate");
}

#[test]
fn checkout_ref_resolution_prefers_override_then_default_and_discovers_remote_head() {
    let state_dir = unique_test_dir("acp-workspace-checkout-ref-resolution");
    let (remote_url, _) = create_bare_remote_repo("acp-workspace-checkout-remote-head");

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
        resolve_https_checkout_ref(&remote_url, None, None, &state_dir)
            .expect("remote HEAD should resolve"),
        Some(format!("refs/heads/{TEST_BRANCH}"))
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
fn workspace_branch_refs_filter_non_branch_refs_and_sort_values() {
    let branches = workspace_branches_from_refs([
        "refs/tags/v1",
        "refs/heads/release",
        "refs/heads/main",
        "refs/heads/main",
    ]);

    assert_eq!(
        branches
            .iter()
            .map(|branch| (branch.name.as_str(), branch.ref_name.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("main", "refs/heads/main"),
            ("release", "refs/heads/release"),
        ]
    );
}

#[test]
fn workspace_branch_preference_moves_default_ref_to_the_front() {
    let mut branches = workspace_branches_from_refs([
        "refs/heads/release",
        "refs/heads/main",
        "refs/heads/hotfix",
    ]);

    prioritize_workspace_branch_ref(&mut branches, Some("refs/heads/main"));

    assert_eq!(
        branches
            .iter()
            .map(|branch| branch.ref_name.as_str())
            .collect::<Vec<_>>(),
        vec!["refs/heads/main", "refs/heads/hotfix", "refs/heads/release"]
    );
}

#[test]
fn local_workspace_branch_listing_reads_local_repository_heads() {
    let repo_dir = unique_test_dir("acp-workspace-branch-list-local");
    initialize_local_repo(&repo_dir);
    let repo = Repository::open(&repo_dir).expect("repo should open");
    let head_commit = repo
        .head()
        .expect("repo should have HEAD")
        .peel_to_commit()
        .expect("HEAD should resolve to a commit");
    repo.branch("release", &head_commit, false)
        .expect("release branch should be creatable");

    let branches = list_local_workspace_branches(
        &repo_dir,
        &unique_test_dir("acp-workspace-branch-list-local-state"),
    )
    .expect("local branches should list");

    assert_eq!(
        branches
            .iter()
            .map(|branch| branch.ref_name.as_str())
            .collect::<Vec<_>>(),
        vec!["refs/heads/test-branch", "refs/heads/release"]
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
fn workspace_checkout_errors_display_their_messages() {
    let error = WorkspaceCheckoutError::Git("git failed".to_string());

    assert_eq!(error.to_string(), "git failed");
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
    std::fs::write(&checkout_path, "stale file").expect("stale checkout marker should be writable");
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

    assert_git_error(error);
    assert!(
        !state_dir.join("session-checkouts/s_test").exists(),
        "failed https preparations should remove partial checkouts"
    );
}

#[test]
fn https_checkout_failures_after_fetch_cleanup_checkout_directories() {
    let state_dir = unique_test_dir("acp-workspace-checkout-https-fetch-failure");
    let manager = FsWorkspaceCheckoutManager::new(state_dir.clone());
    let workspace = sample_workspace_record(
        Some("https://127.0.0.1:9/repo.git"),
        Some("refs/heads/main"),
    );

    let error = manager
        .prepare_checkout_sync(&workspace, "s_fetch", None)
        .expect_err("fetch failures should surface through the git2 checkout path");

    assert_git_error(error);
    assert!(
        !state_dir.join("session-checkouts/s_fetch").exists(),
        "failed preparations should remove the checkout directory"
    );
}

#[test]
fn remote_checkout_helpers_cover_file_url_fetch_and_head_resolution() {
    let state_dir = unique_test_dir("acp-workspace-checkout-remote-state");
    let (remote_url, expected_head) = create_bare_remote_repo("acp-workspace-checkout-remote");
    let checkout_path = unique_test_dir("acp-workspace-checkout-remote-clone");

    let checkout_commit_sha = clone_remote_workspace(&remote_url, None, &checkout_path, &state_dir)
        .expect("file-url remotes should clone");

    assert_eq!(checkout_commit_sha, Some(expected_head.clone()));
    assert_eq!(
        checkout_head_commit(&checkout_path, &state_dir).expect("HEAD should resolve"),
        Some(expected_head)
    );
    assert!(checkout_path.join("fixture.txt").exists());
}

#[test]
fn local_checkout_helpers_cover_clone_checkout_and_commit_resolution() {
    let source_root = unique_test_dir("acp-workspace-checkout-source");
    let expected_head = initialize_local_repo(&source_root);
    let state_dir = unique_test_dir("acp-workspace-checkout-state");
    let checkout_path = unique_test_dir("acp-workspace-checkout-clone");

    let checkout_commit_sha =
        clone_local_repository(&source_root, Some("v1"), &checkout_path, &state_dir)
            .expect("local repositories should clone");
    assert_eq!(checkout_commit_sha, Some(expected_head.clone()));
    assert_eq!(
        checkout_head_commit(&checkout_path, &state_dir).expect("head commits should resolve"),
        Some(expected_head.clone())
    );

    fetch_checkout_origin_head(&checkout_path);
    checkout_fetch_head(&checkout_path, &state_dir).expect("FETCH_HEAD should be check-outable");
    assert_eq!(
        checkout_head_commit(&checkout_path, &state_dir).expect("detached commits should resolve"),
        Some(expected_head)
    );
}

#[test]
fn local_checkout_helpers_preserve_detached_source_heads() {
    let source_root = unique_test_dir("acp-workspace-checkout-detached-source");
    let expected_head = initialize_detached_head_only_repo(&source_root);
    let state_dir = unique_test_dir("acp-workspace-checkout-detached-state");
    let checkout_path = unique_test_dir("acp-workspace-checkout-detached-clone");

    assert_eq!(
        resolve_local_checkout_ref(&source_root, None, None, &state_dir)
            .expect("detached source HEADs should not error"),
        None
    );
    let checkout_commit_sha =
        clone_local_repository(&source_root, None, &checkout_path, &state_dir)
            .expect("detached local HEADs should clone");

    assert_eq!(checkout_commit_sha, Some(expected_head.clone()));
    assert_eq!(
        checkout_head_commit(&checkout_path, &state_dir).expect("HEAD should resolve"),
        Some(expected_head)
    );
    assert!(checkout_path.join("detached.txt").exists());
}

#[test]
fn clone_local_repository_reports_state_dir_creation_failures() {
    let source_root = unique_test_dir("acp-workspace-checkout-local-state-source");
    initialize_local_repo(&source_root);
    let broken_state_dir = unique_test_dir("acp-workspace-checkout-local-state-broken");
    std::fs::create_dir_all(
        broken_state_dir
            .parent()
            .expect("state dir should have a parent"),
    )
    .expect("test parent should be creatable");
    std::fs::write(&broken_state_dir, "state file").expect("state dir blocker should be writable");

    let error = clone_local_repository(
        &source_root,
        None,
        &unique_test_dir("acp-workspace-checkout-local-state-clone"),
        &broken_state_dir,
    )
    .expect_err("file-backed state dirs should fail");

    assert_io_error_contains(error, "creating git home failed");
}

#[test]
fn parse_remote_default_branch_name_rejects_invalid_utf8() {
    assert_eq!(
        parse_remote_default_branch_name(Some("refs/heads/main"))
            .expect("UTF-8 branch names should parse"),
        Some("refs/heads/main".to_string())
    );
    assert_eq!(
        parse_remote_default_branch_name(None).expect_err("missing UTF-8 should fail"),
        WorkspaceCheckoutError::Git("remote default branch is not valid UTF-8".to_string())
    );
}

#[test]
fn resolve_remote_head_ref_reports_probe_directory_creation_failures() {
    let state_dir = state_dir_with_probe_path_overflow("acp-workspace-checkout-probe-overflow");

    let error = resolve_remote_head_ref("file:///unused.git", &state_dir)
        .expect_err("overflowed probe paths should fail before remote access");

    assert_io_error_contains(error, "creating git home failed");
}

#[test]
fn reject_git_credentials_fails_closed() {
    let error = match reject_git_credentials(
        "https://example.com/repo.git",
        None,
        CredentialType::USER_PASS_PLAINTEXT,
    ) {
        Ok(_) => panic!("credential requests should always fail closed"),
        Err(error) => error,
    };

    assert_eq!(
        error.message(),
        "credentialed git transports are not supported"
    );
}

#[test]
fn git_symbolic_ref_handles_branch_detached_unborn_invalid_and_io_failures() {
    let repo = unique_test_dir("acp-workspace-checkout-symbolic-ref");
    initialize_local_repo(&repo);
    let state_dir = unique_test_dir("acp-workspace-checkout-symbolic-state");

    assert_eq!(
        git_symbolic_ref(&repo, &state_dir).expect("branch heads should resolve"),
        Some(format!("refs/heads/{TEST_BRANCH}"))
    );

    detach_head(&repo);
    assert_eq!(
        git_symbolic_ref(&repo, &state_dir).expect("detached heads should not error"),
        None
    );

    let unborn_repo = unique_test_dir("acp-workspace-checkout-symbolic-unborn");
    initialize_unborn_repo(&unborn_repo);
    assert_eq!(
        git_symbolic_ref(
            &unborn_repo,
            &unique_test_dir("acp-workspace-checkout-symbolic-unborn-state")
        )
        .expect("unborn heads should not error"),
        None
    );

    corrupt_head_file(&repo, b"ref: refs/heads/invalid lock\n");
    let invalid_head_error = git_symbolic_ref(
        &repo,
        &unique_test_dir("acp-workspace-checkout-symbolic-invalid-state"),
    )
    .expect_err("invalid HEAD refs should surface git errors");
    assert_git_error(invalid_head_error);

    let broken_state_dir = unique_test_dir("acp-workspace-checkout-symbolic-state-broken");
    std::fs::write(&broken_state_dir, "state file").expect("state dir blocker should be writable");
    let error = git_symbolic_ref(&repo, &broken_state_dir)
        .expect_err("broken state dirs should surface io failures");
    assert_io_error_contains(error, "creating git home failed");
}

#[test]
fn local_source_root_from_returns_git_errors_outside_repositories() {
    let error = local_source_root_from(
        Path::new("/"),
        &unique_test_dir("acp-workspace-checkout-local-root-missing-state"),
    )
    .expect_err("non-repository paths should fail");

    assert_git_error(error);
}

#[test]
fn local_source_root_from_rejects_bare_repositories() {
    let repo = unique_test_dir("acp-workspace-checkout-local-root-bare");
    let mut options = RepositoryInitOptions::new();
    options.bare(true);
    options.external_template(false);
    Repository::init_opts(&repo, &options).expect("bare repositories should initialize");

    let error = local_source_root_from(
        &repo,
        &unique_test_dir("acp-workspace-checkout-local-root-bare-state"),
    )
    .expect_err("bare repositories should not expose a working directory");

    assert_eq!(
        error,
        WorkspaceCheckoutError::Git("repository root has no working directory".to_string())
    );
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
fn checkout_head_commit_returns_none_for_unborn_repositories() {
    let repo_dir = unique_test_dir("acp-workspace-checkout-unborn");
    initialize_unborn_repo(&repo_dir);

    assert_eq!(
        checkout_head_commit(
            &repo_dir,
            &unique_test_dir("acp-workspace-checkout-unborn-state")
        )
        .expect("unborn repos should not error"),
        None
    );
}

#[test]
fn current_head_commit_returns_none_for_missing_head_targets() {
    let repo_dir = unique_test_dir("acp-workspace-checkout-missing-head-target");
    initialize_local_repo(&repo_dir);
    let repo = Repository::open(&repo_dir).expect("repo should open");
    let missing =
        Oid::from_str("0123456789abcdef0123456789abcdef01234567").expect("test OIDs should parse");
    std::fs::write(
        repo.path().join("refs/heads/missing-target"),
        format!("{missing}\n"),
    )
    .expect("direct refs should be writable");
    repo.set_head("refs/heads/missing-target")
        .expect("HEAD should point at the missing object ref");

    assert_eq!(
        current_head_commit(&repo).expect("missing head targets should map to None"),
        None
    );
}

#[test]
fn current_head_commit_reports_non_commit_and_invalid_head_errors() {
    let blob_repo_dir = unique_test_dir("acp-workspace-checkout-blob-head");
    initialize_local_repo(&blob_repo_dir);
    let blob_repo = Repository::open(&blob_repo_dir).expect("repo should open");
    let blob_id = blob_repo
        .blob(b"blob-only")
        .expect("test blobs should be writable");
    blob_repo
        .reference("refs/heads/blob-head", blob_id, true, "test")
        .expect("blob refs should be writable");
    blob_repo
        .set_head("refs/heads/blob-head")
        .expect("HEAD should point at the blob ref");

    let blob_error =
        current_head_commit(&blob_repo).expect_err("blob heads should not peel to commits");
    assert_git_error(blob_error);

    let invalid_repo_dir = unique_test_dir("acp-workspace-checkout-invalid-head");
    initialize_local_repo(&invalid_repo_dir);
    corrupt_head_file(&invalid_repo_dir, b"ref: refs/heads/invalid lock\n");
    let invalid_repo = Repository::open(&invalid_repo_dir).expect("repo should still open");

    let invalid_head_error =
        current_head_commit(&invalid_repo).expect_err("invalid HEAD refs should error");
    assert_git_error(invalid_head_error);
}

#[test]
fn map_git_error_falls_back_for_blank_messages() {
    let error = map_git_error(git2::Error::new(
        ErrorCode::GenericError,
        ErrorClass::Net,
        "   ",
    ));

    assert_eq!(
        error,
        WorkspaceCheckoutError::Git("git operation failed (GenericError)".to_string())
    );
}

#[test]
fn local_source_root_reports_deleted_current_directories() {
    const CHILD_ENV: &str = "ACP_WORKSPACE_CHECKOUT_DELETED_CWD_CHILD";
    if std::env::var_os(CHILD_ENV).is_some() {
        let deleted_dir =
            PathBuf::from(std::env::var("ACP_DELETED_CWD").expect("deleted cwd env should exist"));
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
        "workspace_checkout::tests::checkout::local_source_root_reports_deleted_current_directories",
        &[
            (CHILD_ENV, "1"),
            ("ACP_DELETED_CWD", deleted_dir_string.as_str()),
        ],
    );
    assert_child_success(output);
}

#[test]
fn clone_local_repository_propagates_git_failures_for_unknown_refs() {
    let repo = unique_test_dir("acp-workspace-checkout-local-ref-failure");
    initialize_local_repo(&repo);

    let error = clone_local_repository(
        &repo,
        Some("refs/heads/missing"),
        &unique_test_dir("acp-workspace-checkout-local-ref-checkout"),
        &unique_test_dir("acp-workspace-checkout-local-ref-state"),
    )
    .expect_err("unknown local refs should fail checkout");

    assert_git_error(error);
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

#[test]
fn local_workspace_fallback_prepares_a_checkout_from_detached_head_sources() {
    const CHILD_ENV: &str = "ACP_WORKSPACE_CHECKOUT_DETACHED_HEAD_CHILD";
    const SOURCE_ENV: &str = "ACP_WORKSPACE_CHECKOUT_DETACHED_HEAD_SOURCE";
    const EXPECTED_ENV: &str = "ACP_WORKSPACE_CHECKOUT_DETACHED_HEAD_EXPECTED";

    if std::env::var_os(CHILD_ENV).is_some() {
        let source_root = PathBuf::from(
            std::env::var(SOURCE_ENV).expect("detached source root env should exist"),
        );
        let expected_commit =
            std::env::var(EXPECTED_ENV).expect("detached source commit env should exist");
        let original_dir = std::env::current_dir().expect("current dir should be readable");
        std::env::set_current_dir(&source_root)
            .expect("child should be able to chdir into the detached source");
        let state_dir = source_root
            .parent()
            .expect("detached source should have a parent")
            .join(format!(
                "acp-workspace-checkout-detached-head-state-{}",
                uuid::Uuid::new_v4().simple()
            ));
        let manager = FsWorkspaceCheckoutManager::new(state_dir);
        let checkout = manager
            .prepare_checkout_sync(&sample_workspace_record(None, None), "s_detached", None)
            .expect("detached source local checkout should prepare");

        assert_eq!(checkout.checkout_ref, None);
        assert_eq!(checkout.checkout_commit_sha, Some(expected_commit));
        assert!(checkout.working_dir.join("detached.txt").exists());
        std::env::set_current_dir(original_dir)
            .expect("child should restore the original working directory");
        return;
    }

    let source_root = unique_test_dir("acp-workspace-checkout-detached-head-source");
    let expected_commit = initialize_detached_head_only_repo(&source_root);
    let source_root_string = source_root.to_string_lossy().to_string();
    let output = run_self_test_child(
        "workspace_checkout::tests::checkout::local_workspace_fallback_prepares_a_checkout_from_detached_head_sources",
        &[
            (CHILD_ENV, "1"),
            (SOURCE_ENV, source_root_string.as_str()),
            (EXPECTED_ENV, expected_commit.as_str()),
        ],
    );
    assert_child_success(output);
}
