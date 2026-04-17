use super::*;
use acp_app_support::{init_tracing, unique_temp_json_path};
use std::{
    ffi::OsString,
    fs,
    os::unix::{ffi::OsStringExt, fs::PermissionsExt},
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    process::Command,
};

#[test]
fn launcher_state_path_uses_home_dir_without_data_dir() {
    let path = launcher_state_path_from(None, None, Some(PathBuf::from("/tmp/home")))
        .expect("home directory fallback should resolve");

    assert_eq!(
        path,
        PathBuf::from("/tmp/home/.acp-orchestrator/launcher-stack.json")
    );
}

#[tokio::test]
async fn shutdown_terminates_optional_mock_children() {
    let mut stack = LauncherStack::ephemeral(
        spawn_sleep_child().await,
        Some(spawn_sleep_child().await),
        "http://127.0.0.1:1".to_string(),
        "launcher-auth-token".to_string(),
    );

    stack
        .shutdown()
        .await
        .expect("ephemeral launcher stacks should shut down cleanly");

    assert!(stack.ephemeral_children.is_none());
}

#[tokio::test]
async fn launcher_stacks_report_when_children_are_ephemeral() {
    let persistent = LauncherStack::persistent(
        "http://127.0.0.1:1".to_string(),
        "launcher-auth-token".to_string(),
    );
    let mut ephemeral = LauncherStack::ephemeral(
        spawn_sleep_child().await,
        None,
        "http://127.0.0.1:1".to_string(),
        "launcher-auth-token".to_string(),
    );

    assert!(!persistent.is_ephemeral());
    assert!(ephemeral.is_ephemeral());

    ephemeral
        .shutdown()
        .await
        .expect("ephemeral launcher stacks should shut down cleanly");
}

#[tokio::test]
async fn prepare_launcher_stack_uses_direct_mode_with_acp_server_url_env() {
    let _guard = crate::tests::test_env_lock().lock().await;
    let _url_guard = crate::tests::test_acp_server_url_guard(Some("http://127.0.0.1:8080"));
    let args = LauncherArgs {
        acp_server: None,
        web: false,
        cli_args: vec![OsString::from("chat"), OsString::from("--new")],
    };

    let stack = prepare_launcher_stack(Path::new("/bin/true"), &args, true, false, None)
        .await
        .expect("explicit ACP_SERVER_URL should skip launcher-managed services");

    assert!(stack.backend_url().is_none());
}

#[test]
fn launcher_lock_drop_tolerates_non_file_paths() {
    init_tracing();
    let path = unique_temp_json_path("acp-launcher-lock", "drop-directory");
    fs::create_dir_all(&path).expect("lock path directory should be creatable");

    drop(LauncherLock { path });
}

#[test]
fn try_acquire_launcher_lock_records_the_owner_pid() {
    let lock_path = unique_temp_json_path("acp-launcher-lock", "owner-pid");

    let lock = try_acquire_launcher_lock(&lock_path)
        .expect("creating the launcher lock should succeed")
        .expect("the lock should be acquired");

    assert_eq!(
        fs::read_to_string(&lock_path).expect("launcher lock should be readable"),
        std::process::id().to_string()
    );

    drop(lock);
}

#[test]
fn write_launcher_lock_owner_removes_the_lock_when_recording_the_owner_fails() {
    #[derive(Default)]
    struct FailingWriter;

    impl std::io::Write for FailingWriter {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("boom"))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    let lock_path = unique_temp_json_path("acp-launcher-lock", "owner-write-failure");
    fs::write(&lock_path, []).expect("launcher lock placeholder should write");

    let error = write_launcher_lock_owner(&mut FailingWriter, &lock_path)
        .expect_err("owner write failures should be surfaced");

    assert!(matches!(
        error,
        crate::LauncherError::AcquireLauncherLock { .. }
    ));
    assert!(!lock_path.exists());
}

#[tokio::test]
async fn prepare_persistent_stack_times_out_when_the_lock_stays_busy() {
    let state_path = unique_temp_json_path("acp-launcher-state", "busy-lock");
    let lock_path = launcher_lock_path_from(&state_path);
    let lock = try_acquire_launcher_lock(&lock_path)
        .expect("creating the launcher lock should succeed")
        .expect("the lock should be acquired");

    let error = prepare_persistent_bundled_stack_with_retry(
        Path::new("/bin/true"),
        &state_path,
        &test_launcher_identity("busy-lock"),
        1,
        Duration::ZERO,
        Duration::from_secs(3600),
        None,
    )
    .await
    .expect_err("busy launcher locks should eventually time out");

    drop(lock);
    assert!(matches!(
        error,
        crate::LauncherError::WaitForLauncherLock { .. }
    ));
}

#[tokio::test]
async fn prepare_persistent_stack_clears_stale_locks_before_timing_out() {
    let state_path = unique_temp_json_path("acp-launcher-state", "stale-lock");
    let lock_path = launcher_lock_path_from(&state_path);
    fs::write(&lock_path, []).expect("stale lock files should be creatable");

    let error = prepare_persistent_bundled_stack_with_retry(
        Path::new("/bin/true"),
        &state_path,
        &test_launcher_identity("stale-lock"),
        1,
        Duration::ZERO,
        Duration::ZERO,
        None,
    )
    .await
    .expect_err("without reusable state the launcher should still time out");

    assert!(matches!(
        error,
        crate::LauncherError::WaitForLauncherLock { .. }
    ));
    assert!(!lock_path.exists());
}

#[tokio::test]
async fn prepare_persistent_stack_uses_the_final_reuse_check() {
    let state_path = unique_temp_json_path("acp-launcher-state", "final-reuse");
    let (backend_url, health_task) = spawn_health_server().await;
    let mock_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("mock listener should bind");
    save_launcher_state(
        &state_path,
        &test_launcher_state(
            &backend_url,
            Some(
                &mock_listener
                    .local_addr()
                    .expect("mock listener address should be readable")
                    .to_string(),
            ),
        ),
    )
    .expect("launcher state should save");

    let stack = prepare_persistent_bundled_stack_with_retry(
        Path::new("/bin/true"),
        &state_path,
        &test_launcher_identity("current"),
        0,
        Duration::ZERO,
        Duration::ZERO,
        None,
    )
    .await
    .expect("the final reuse check should return the healthy stack");

    health_task.abort();
    assert_eq!(stack.backend_url(), Some(backend_url.as_str()));
    assert_eq!(stack.auth_token(), Some("launcher-auth-token"));
}

#[tokio::test]
async fn spawn_or_reuse_locked_stack_reuses_existing_state() {
    let state_path = unique_temp_json_path("acp-launcher-state", "locked-reuse");
    let lock_path = launcher_lock_path_from(&state_path);
    let lock = try_acquire_launcher_lock(&lock_path)
        .expect("creating the launcher lock should succeed")
        .expect("the lock should be acquired");
    let (backend_url, health_task) = spawn_health_server().await;
    let mock_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("mock listener should bind");
    save_launcher_state(
        &state_path,
        &test_launcher_state(
            &backend_url,
            Some(
                &mock_listener
                    .local_addr()
                    .expect("mock listener address should be readable")
                    .to_string(),
            ),
        ),
    )
    .expect("launcher state should save");

    let stack = spawn_or_reuse_locked_stack(
        Path::new("/bin/true"),
        &state_path,
        &test_launcher_identity("current"),
        lock,
        None,
    )
    .await
    .expect("the existing healthy stack should be reused");

    health_task.abort();
    assert_eq!(stack.backend_url(), Some(backend_url.as_str()));
    assert_eq!(stack.auth_token(), Some("launcher-auth-token"));
}

#[tokio::test]
async fn reusable_launcher_state_clears_invalid_json_without_a_lock() {
    let state_path = unique_temp_json_path("acp-launcher-state", "invalid-json");
    fs::write(&state_path, "{invalid").expect("invalid launcher state should write");

    assert_eq!(
        reusable_launcher_state(&state_path, &test_launcher_identity("current"), None)
            .await
            .expect("invalid json should be ignored"),
        None
    );
    assert!(!state_path.exists());
}

#[tokio::test]
async fn reusable_launcher_state_keeps_invalid_json_while_the_lock_exists() {
    let state_path = unique_temp_json_path("acp-launcher-state", "invalid-json-locked");
    let lock_path = launcher_lock_path_from(&state_path);
    fs::write(&state_path, "{invalid").expect("invalid launcher state should write");
    fs::write(&lock_path, []).expect("launcher lock should write");

    assert_eq!(
        reusable_launcher_state(&state_path, &test_launcher_identity("current"), None)
            .await
            .expect("invalid json should be ignored while locked"),
        None
    );
    assert!(state_path.exists());
}

#[tokio::test]
async fn reusable_launcher_state_propagates_non_parse_read_errors() {
    let state_path = unique_temp_json_path("acp-launcher-state", "read-error");
    fs::create_dir_all(&state_path).expect("state path directory should be creatable");

    let error = reusable_launcher_state(&state_path, &test_launcher_identity("current"), None)
        .await
        .expect_err("non-parse read failures should still be surfaced");

    assert!(matches!(
        error,
        crate::LauncherError::ReadLauncherState { .. }
    ));
}

#[tokio::test]
async fn reusable_launcher_state_treats_invalid_backend_urls_as_unhealthy() {
    let state_path = unique_temp_json_path("acp-launcher-state", "invalid-backend-url");
    let mock_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("mock listener should bind");
    save_launcher_state(
        &state_path,
        &test_launcher_state(
            "://invalid",
            Some(
                &mock_listener
                    .local_addr()
                    .expect("mock listener address should be readable")
                    .to_string(),
            ),
        ),
    )
    .expect("launcher state should save");

    assert_eq!(
        reusable_launcher_state(&state_path, &test_launcher_identity("current"), None)
            .await
            .expect("invalid backend urls should be ignored"),
        None
    );
    assert!(state_path.exists());
}

#[tokio::test]
async fn reusable_launcher_state_rejects_identity_mismatches() {
    let state_path = unique_temp_json_path("acp-launcher-state", "identity-mismatch");
    let (backend_url, health_task) = spawn_health_server().await;
    let mock_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("mock listener should bind");
    save_launcher_state(
        &state_path,
        &test_launcher_state_with_identity(
            &backend_url,
            Some(
                &mock_listener
                    .local_addr()
                    .expect("mock listener address should be readable")
                    .to_string(),
            ),
            test_launcher_identity("old-binary"),
        ),
    )
    .expect("launcher state should save");

    assert_eq!(
        reusable_launcher_state(&state_path, &test_launcher_identity("new-binary"), None)
            .await
            .expect("identity mismatches should be ignored"),
        None
    );

    health_task.abort();
}

#[tokio::test]
async fn reusable_launcher_state_rejects_frontend_dist_mismatches() {
    let state_path = unique_temp_json_path("acp-launcher-state", "frontend-mismatch");
    let requested_frontend_dist =
        unique_temp_json_path("acp-launcher-frontend", "requested").with_extension("");
    let stored_frontend_dist =
        unique_temp_json_path("acp-launcher-frontend", "stored").with_extension("");
    let mut state = test_launcher_state("http://127.0.0.1:1", Some("127.0.0.1:1"));
    state.frontend_dist = Some(path_to_string(&stored_frontend_dist));
    save_launcher_state(&state_path, &state).expect("launcher state should save");

    assert_eq!(
        reusable_launcher_state(
            &state_path,
            &test_launcher_identity("current"),
            Some(&requested_frontend_dist),
        )
        .await
        .expect("frontend mismatches should be ignored"),
        None
    );
}

#[test]
fn backend_role_args_include_frontend_dist_when_requested() {
    let frontend_dist = Path::new("/tmp/acp-frontend-dist");
    let args = backend_role_args(OsString::from("127.0.0.1:8090"), true, Some(frontend_dist));

    assert!(args.windows(2).any(|window| {
        window
            == [
                OsString::from("--frontend-dist"),
                frontend_dist.as_os_str().to_owned(),
            ]
    }));
}

#[test]
fn backend_role_args_omit_startup_hints_when_disabled() {
    let args = backend_role_args(OsString::from("127.0.0.1:8090"), false, None);

    assert!(!args.iter().any(|arg| arg == "--startup-hints"));
}

#[test]
fn mock_role_args_omit_startup_hints_when_disabled() {
    let args = mock_role_args(false);

    assert!(!args.iter().any(|arg| arg == "--startup-hints"));
}

#[test]
fn launcher_state_supports_frontend_requires_matching_dist_paths() {
    let frontend_dist = Path::new("/tmp/acp-frontend-dist");
    let mut state = test_launcher_state("http://127.0.0.1:1", Some("127.0.0.1:1"));
    state.frontend_dist = Some(path_to_string(frontend_dist));

    assert!(launcher_state_supports_frontend(
        &state,
        Some(frontend_dist)
    ));
    assert!(!launcher_state_supports_frontend(
        &state,
        Some(Path::new("/tmp/other-frontend-dist")),
    ));
}

#[test]
fn warn_and_maybe_clear_invalid_launcher_state_tolerates_directory_paths() {
    init_tracing();
    let state_path = unique_temp_json_path("acp-launcher-state", "invalid-directory");
    fs::create_dir_all(&state_path).expect("state path directory should be creatable");

    warn_and_maybe_clear_invalid_launcher_state(
        &state_path,
        &parse_launcher_state_error(&state_path),
    );

    assert!(state_path.exists());
}

#[test]
fn try_acquire_launcher_lock_reports_open_failures() {
    let lock_path = PathBuf::from(OsString::from_vec(b"launcher\0lock".to_vec()));

    let error =
        try_acquire_launcher_lock(&lock_path).expect_err("invalid lock paths should fail to open");

    assert!(matches!(
        error,
        crate::LauncherError::AcquireLauncherLock { .. }
    ));
}

#[test]
fn clear_stale_launcher_lock_handles_not_stale_files() {
    let lock_path = unique_temp_json_path("acp-launcher-lock", "not-stale");
    fs::write(&lock_path, []).expect("launcher lock should write");

    assert!(
        !clear_stale_launcher_lock(&lock_path, Duration::from_secs(3600))
            .expect("fresh launcher locks should be retained")
    );
    assert!(lock_path.exists());
}

#[test]
fn clear_stale_launcher_lock_clears_dead_owner_locks_immediately() {
    let lock_path = unique_temp_json_path("acp-launcher-lock", "dead-owner");
    let mut child = std::process::Command::new("sh")
        .arg("-c")
        .arg(":")
        .spawn()
        .expect("dead owner helper process should spawn");
    let dead_owner_pid = child.id();
    child
        .wait()
        .expect("dead owner helper process should exit cleanly");
    fs::write(&lock_path, dead_owner_pid.to_string()).expect("launcher lock should write");

    assert!(
        clear_stale_launcher_lock(&lock_path, Duration::from_secs(3600))
            .expect("dead owner launcher locks should be removed immediately")
    );
    assert!(!lock_path.exists());
}

#[test]
fn clear_stale_launcher_lock_reports_metadata_failures() {
    let lock_path = path_under_file_parent("metadata-failure", "lock");

    let error = clear_stale_launcher_lock(&lock_path, Duration::ZERO)
        .expect_err("metadata failures should be surfaced");

    assert!(matches!(
        error,
        crate::LauncherError::ReadLauncherLockMetadata { .. }
    ));
}

#[test]
fn clear_stale_launcher_lock_reports_remove_failures() {
    let lock_path = unique_temp_json_path("acp-launcher-lock", "remove-failure");
    fs::create_dir_all(&lock_path).expect("lock path directory should be creatable");

    let error = clear_stale_launcher_lock(&lock_path, Duration::ZERO)
        .expect_err("directory lock paths should fail to remove");

    assert!(matches!(
        error,
        crate::LauncherError::RemoveLauncherLock { .. }
    ));
}

#[tokio::test]
async fn spawn_persistent_bundled_backend_handles_backend_spawn_failures() {
    let script = write_fake_launcher_script(
        r#"#!/bin/sh
if [ "$2" = "mock" ]; then
  echo "acp mock listening on 127.0.0.1:65535"
  sleep 1
else
  exit 1
fi
"#,
    );

    let error =
        spawn_persistent_bundled_backend(&script, &test_launcher_identity("spawn-failure"), None)
            .await
            .expect_err("backend startup failures should be returned");

    assert!(matches!(
        error,
        crate::LauncherError::InvalidStartupLine { .. }
    ));
}

#[tokio::test]
async fn persist_launcher_state_or_shutdown_stops_children_when_saving_fails() {
    let mut backend = spawn_sleep_child().await;
    let mut mock = spawn_sleep_child().await;
    let state_path = path_under_file_parent("save-error", "launcher-stack.json");

    let error = persist_launcher_state_or_shutdown(
        &state_path,
        &test_launcher_state("http://127.0.0.1:1", Some("127.0.0.1:1")),
        &mut backend,
        &mut mock,
    )
    .await
    .expect_err("invalid state paths should fail to save");

    assert!(matches!(
        error,
        crate::LauncherError::CreateLauncherStateDirectory { .. }
    ));
    assert!(
        backend
            .try_wait()
            .expect("backend child should be queryable")
            .is_some()
    );
    assert!(
        mock.try_wait()
            .expect("mock child should be queryable")
            .is_some()
    );
}

#[test]
fn save_launcher_state_creates_parent_directories() {
    let state_path = unique_temp_json_path("acp-launcher-state", "nested-parent")
        .with_extension("")
        .join("launcher-stack.json");
    save_launcher_state(
        &state_path,
        &test_launcher_state("http://127.0.0.1:1", Some("127.0.0.1:1")),
    )
    .expect("saving launcher state should create parent directories");

    assert!(state_path.exists());
    #[cfg(unix)]
    assert_eq!(
        fs::metadata(&state_path)
            .expect("state metadata should load")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[cfg(unix)]
#[test]
fn save_launcher_state_secures_default_launcher_directories_on_unix() {
    let state_path = unique_temp_json_path("acp-launcher-state-root", "secure-parent")
        .with_extension("")
        .join(".acp-orchestrator")
        .join("launcher-stack.json");
    save_launcher_state(
        &state_path,
        &test_launcher_state("http://127.0.0.1:1", Some("127.0.0.1:1")),
    )
    .expect("saving launcher state should secure default launcher directories");

    assert_eq!(
        fs::metadata(
            state_path
                .parent()
                .expect("launcher state paths should have a parent"),
        )
        .expect("parent directory metadata should load")
        .permissions()
        .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(&state_path)
            .expect("state metadata should load")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn create_launcher_state_parent_creates_nested_directories() {
    let state_path = unique_temp_json_path("acp-launcher-state", "parent-helper")
        .with_extension("")
        .join("launcher-stack.json");

    create_launcher_state_parent(&state_path)
        .expect("creating a nested launcher state parent should succeed");

    assert!(
        state_path
            .parent()
            .expect("nested launcher state should have a parent")
            .exists()
    );
}

#[test]
fn create_launcher_state_parent_skips_paths_without_a_parent_component() {
    create_launcher_state_parent(Path::new("launcher-stack.json"))
        .expect("plain file names should not require directory creation");
}

#[tokio::test]
async fn managed_stack_is_healthy_requires_a_mock_address() {
    assert!(
        !managed_stack_is_healthy(&LauncherState {
            mock_address: None,
            ..test_launcher_state("http://127.0.0.1:1", Some("127.0.0.1:1"))
        })
        .await
    );
}

#[tokio::test]
async fn managed_stack_is_healthy_rejects_dead_mock_endpoints() {
    assert!(
        !managed_stack_is_healthy(&LauncherState {
            mock_address: Some("127.0.0.1:9".to_string()),
            ..test_launcher_state("http://127.0.0.1:1", Some("127.0.0.1:1"))
        })
        .await
    );
}

#[tokio::test]
async fn managed_stack_is_healthy_rejects_non_loopback_backend_urls() {
    assert!(
        !managed_stack_is_healthy(&test_launcher_state(
            "http://example.com",
            Some("127.0.0.1:9")
        ))
        .await
    );
}

#[tokio::test]
async fn managed_stack_is_healthy_checks_frontend_assets_for_web_stacks() {
    let (backend_url, health_task) = spawn_health_server().await;
    let mock_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("mock listener should bind");
    let mut state = test_launcher_state(
        &backend_url,
        Some(
            &mock_listener
                .local_addr()
                .expect("mock listener address should be readable")
                .to_string(),
        ),
    );
    state.frontend_dist = Some("/tmp/acp-frontend-dist".to_string());

    assert!(managed_stack_is_healthy(&state).await);

    health_task.abort();
}

#[test]
fn socket_address_uses_loopback_accepts_bracketed_ipv6_loopback() {
    assert!(socket_address_uses_loopback("[::1]:8080"));
}

fn parse_launcher_state_error(path: &Path) -> crate::LauncherError {
    crate::LauncherError::ParseLauncherState {
        source: serde_json::from_str::<LauncherState>("{invalid")
            .expect_err("invalid json should fail to parse"),
        path: path.to_path_buf(),
    }
}

fn test_launcher_identity(label: &str) -> LauncherIdentity {
    LauncherIdentity {
        executable_path: format!("/bin/{label}"),
        build_fingerprint: format!("fingerprint-{label}"),
    }
}

fn test_launcher_state(backend_url: &str, mock_address: Option<&str>) -> LauncherState {
    test_launcher_state_with_identity(backend_url, mock_address, test_launcher_identity("current"))
}

fn test_launcher_state_with_identity(
    backend_url: &str,
    mock_address: Option<&str>,
    launcher_identity: LauncherIdentity,
) -> LauncherState {
    LauncherState {
        backend_url: backend_url.to_string(),
        mock_address: mock_address.map(str::to_string),
        frontend_dist: None,
        auth_token: "launcher-auth-token".to_string(),
        launcher_identity,
    }
}

fn path_under_file_parent(label: &str, child: &str) -> PathBuf {
    let parent = unique_temp_json_path("acp-launcher-parent", label);
    fs::write(&parent, "file").expect("file parent should write");
    parent.join(child)
}

fn write_fake_launcher_script(contents: &str) -> PathBuf {
    let path = unique_temp_json_path("acp-launcher-script", "mock-backend");
    fs::write(&path, contents).expect("fake launcher script should write");
    let mut permissions = fs::metadata(&path)
        .expect("fake launcher script metadata should load")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).expect("fake launcher script should become executable");
    path
}

async fn spawn_health_server() -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("health listener should bind");
    let address = listener
        .local_addr()
        .expect("health listener address should be readable");
    let task = tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buffer = [0_u8; 1024];
                let _ = stream.read(&mut buffer).await;
                let _ = stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 15\r\nConnection: close\r\n\r\n{\"status\":\"ok\"}",
                    )
                    .await;
            });
        }
    });

    (format!("http://{address}"), task)
}

async fn spawn_sleep_child() -> tokio::process::Child {
    Command::new("sh")
        .arg("-c")
        .arg("sleep 30")
        .spawn()
        .expect("sleep child should spawn")
}
