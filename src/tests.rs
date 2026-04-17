use super::*;
use acp_app_support::{FrontendBundleAsset, frontend_bundle_file_name, unique_temp_json_path};
use std::{
    ffi::OsString,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Stdio,
    sync::OnceLock,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    process::Command,
    sync::{Mutex, MutexGuard},
};

static TEST_ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

pub(crate) fn test_env_lock() -> &'static Mutex<()> {
    TEST_ENV_LOCK.get_or_init(|| Mutex::const_new(()))
}

pub(crate) struct TestAcpServerUrlGuard {
    previous: Option<OsString>,
}

impl Drop for TestAcpServerUrlGuard {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.take() {
            unsafe {
                std::env::set_var("ACP_SERVER_URL", previous);
            }
        } else {
            unsafe {
                std::env::remove_var("ACP_SERVER_URL");
            }
        }
    }
}

pub(crate) fn test_acp_server_url_guard(value: Option<&str>) -> TestAcpServerUrlGuard {
    let previous = std::env::var_os("ACP_SERVER_URL");
    if let Some(value) = value {
        unsafe {
            std::env::set_var("ACP_SERVER_URL", value);
        }
    } else {
        unsafe {
            std::env::remove_var("ACP_SERVER_URL");
        }
    }
    TestAcpServerUrlGuard { previous }
}

struct TestPathGuard {
    previous: Option<OsString>,
}

impl Drop for TestPathGuard {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.take() {
            unsafe {
                std::env::set_var("PATH", previous);
            }
        } else {
            unsafe {
                std::env::remove_var("PATH");
            }
        }
    }
}

fn test_path_guard(value: Option<OsString>) -> TestPathGuard {
    let previous = std::env::var_os("PATH");
    if let Some(value) = value {
        unsafe {
            std::env::set_var("PATH", value);
        }
    } else {
        unsafe {
            std::env::remove_var("PATH");
        }
    }
    TestPathGuard { previous }
}

#[test]
fn split_launcher_args_defaults_to_chat_new() {
    let args = vec![OsString::from("acp")];

    assert_eq!(
        split_launcher_args(&args).expect("default launcher args should parse"),
        LauncherArgs {
            acp_server: None,
            web: false,
            cli_args: vec![OsString::from("chat"), OsString::from("--new")],
        }
    );
}

#[test]
fn split_launcher_args_preserves_explicit_arguments() {
    let args = vec![
        OsString::from("acp"),
        OsString::from("session"),
        OsString::from("list"),
    ];

    assert_eq!(
        split_launcher_args(&args).expect("explicit launcher args should parse"),
        LauncherArgs {
            acp_server: None,
            web: false,
            cli_args: vec![OsString::from("session"), OsString::from("list")],
        }
    );
}

#[test]
fn split_launcher_args_requires_an_acp_server_value() {
    let args = vec![OsString::from("acp"), OsString::from("--acp-server")];

    let error = split_launcher_args(&args).expect_err("missing ACP server values should fail");

    assert!(matches!(error, LauncherError::MissingAcpServer));
}

#[test]
fn split_launcher_args_rejects_an_empty_acp_server_value() {
    let args = vec![
        OsString::from("acp"),
        OsString::from("--acp-server"),
        OsString::from(""),
    ];

    let error = split_launcher_args(&args).expect_err("empty ACP server values should fail");

    assert!(matches!(error, LauncherError::MissingAcpServer));
}

#[test]
fn split_launcher_args_rejects_an_empty_equals_form_acp_server_override() {
    let args = vec![OsString::from("acp"), OsString::from("--acp-server=")];

    let error = split_launcher_args(&args).expect_err("empty ACP server values should fail");

    assert!(matches!(error, LauncherError::MissingAcpServer));
}

#[test]
fn split_launcher_args_extracts_supported_acp_server_overrides() {
    let cases = [
        vec!["acp", "--acp-server", "127.0.0.1:8090"],
        vec!["acp", "--acp-server=127.0.0.1:8090"],
        vec!["acp", "--acp-server", "127.0.0.1:8090", "chat", "--new"],
    ];

    for raw_args in cases {
        let args = raw_args.into_iter().map(OsString::from).collect::<Vec<_>>();

        assert_eq!(
            split_launcher_args(&args).expect("ACP server overrides should parse"),
            LauncherArgs {
                acp_server: Some(OsString::from("127.0.0.1:8090")),
                web: false,
                cli_args: vec![OsString::from("chat"), OsString::from("--new")],
            }
        );
    }
}

#[test]
fn split_launcher_args_extracts_web_mode_without_defaulting_to_cli_chat() {
    let args = vec![OsString::from("acp"), OsString::from("--web")];

    assert_eq!(
        split_launcher_args(&args).expect("web mode should parse"),
        LauncherArgs {
            acp_server: None,
            web: true,
            cli_args: Vec::new(),
        }
    );
}

#[test]
fn split_launcher_args_supports_web_mode_with_an_acp_server_override() {
    let args = vec![
        OsString::from("acp"),
        OsString::from("--web"),
        OsString::from("--acp-server"),
        OsString::from("127.0.0.1:8090"),
    ];

    assert_eq!(
        split_launcher_args(&args).expect("web mode with ACP overrides should parse"),
        LauncherArgs {
            acp_server: Some(OsString::from("127.0.0.1:8090")),
            web: true,
            cli_args: Vec::new(),
        }
    );
}

#[test]
fn web_backend_url_prefers_the_stack_value() {
    let stack = launcher_stack::LauncherStack::persistent(
        "https://127.0.0.1:8443".to_string(),
        "token".to_string(),
    );

    let backend_url = web_backend_url(&stack).expect("stack backend URLs should win");

    assert_eq!(backend_url, "https://127.0.0.1:8443");
}

#[test]
fn web_backend_url_falls_back_to_the_environment() {
    let _guard = lock_acp_server_url();
    let _url_guard = test_acp_server_url_guard(Some("https://127.0.0.1:9443"));

    let backend_url = web_backend_url(&launcher_stack::LauncherStack::direct())
        .expect("environment backend URLs should be used");

    assert_eq!(backend_url, "https://127.0.0.1:9443");
}

#[test]
fn web_backend_url_requires_a_value_from_the_stack_or_environment() {
    let _guard = lock_acp_server_url();
    let _url_guard = test_acp_server_url_guard(None);

    let error = web_backend_url(&launcher_stack::LauncherStack::direct())
        .expect_err("missing backend URLs should fail");

    assert!(matches!(error, LauncherError::MissingBackendUrl));
}

#[test]
fn acp_server_url_guard_restores_previous_values() {
    let _guard = lock_acp_server_url();
    let _original = test_acp_server_url_guard(Some("https://127.0.0.1:1111"));

    {
        let _restore = test_acp_server_url_guard(Some("https://127.0.0.1:2222"));
        assert_eq!(
            std::env::var("ACP_SERVER_URL").ok().as_deref(),
            Some("https://127.0.0.1:2222")
        );
    }

    assert_eq!(
        std::env::var("ACP_SERVER_URL").ok().as_deref(),
        Some("https://127.0.0.1:1111")
    );
}

#[test]
fn command_needs_backend_skips_help_and_version_only() {
    assert!(!command_needs_backend(&[OsString::from("--help")]));
    assert!(!command_needs_backend(&[OsString::from("--version")]));
    assert!(!command_needs_backend(&[
        OsString::from("chat"),
        OsString::from("--help"),
    ]));
    assert!(!command_needs_backend(&[
        OsString::from("session"),
        OsString::from("--help"),
    ]));
    assert!(!command_needs_backend(&[
        OsString::from("session"),
        OsString::from("list"),
        OsString::from("--help"),
    ]));
    assert!(command_needs_backend(&[
        OsString::from("session"),
        OsString::from("list"),
    ]));
    assert!(command_needs_backend(&[
        OsString::from("chat"),
        OsString::from("--new"),
    ]));
    assert!(command_needs_backend(&[
        OsString::from("session"),
        OsString::from("close"),
        OsString::from("s_test"),
    ]));
}

#[test]
fn cli_server_url_is_explicit_accepts_both_supported_forms() {
    assert!(cli_server_url_is_explicit(&[
        OsString::from("chat"),
        OsString::from("--server-url"),
        OsString::from("http://127.0.0.1:8080"),
    ]));
    assert!(cli_server_url_is_explicit(&[
        OsString::from("chat"),
        OsString::from("--server-url=http://127.0.0.1:8080"),
    ]));
    assert!(!cli_server_url_is_explicit(&[
        OsString::from("chat"),
        OsString::from("--new"),
    ]));
}

#[test]
fn launcher_state_path_uses_data_dir_before_home_dir() {
    let path = launcher_state_path_from(
        None,
        Some(PathBuf::from("/tmp/local-data")),
        Some(PathBuf::from("/tmp/home")),
    )
    .expect("data directory paths should resolve");

    assert_eq!(
        path,
        PathBuf::from("/tmp/local-data/acp-orchestrator/launcher-stack.json")
    );
}

#[test]
fn launcher_state_path_uses_home_dir_without_a_data_dir() {
    let path = launcher_state_path_from(None, None, Some(PathBuf::from("/tmp/home")))
        .expect("home directory paths should resolve");

    assert_eq!(
        path,
        PathBuf::from("/tmp/home/.acp-orchestrator/launcher-stack.json")
    );
}

#[test]
fn launcher_state_path_uses_explicit_override_first() {
    let path = launcher_state_path_from(
        Some(OsString::from("/tmp/acp-launcher-state.json")),
        Some(PathBuf::from("/ignored")),
        Some(PathBuf::from("/ignored-home")),
    )
    .expect("explicit launcher state paths should resolve");

    assert_eq!(path, PathBuf::from("/tmp/acp-launcher-state.json"));
}

#[test]
fn launcher_state_path_requires_a_safe_directory_without_overrides() {
    let error = launcher_state_path_from(None, None, None)
        .expect_err("missing directory hints should fail");

    assert!(matches!(
        error,
        LauncherError::MissingLauncherStateDirectory
    ));
}

#[tokio::test]
async fn run_with_args_requires_an_internal_role_name() {
    let error = run_with_args(vec![
        OsString::from("acp"),
        OsString::from("__internal-role"),
    ])
    .await
    .expect_err("missing internal role should fail");

    assert!(matches!(error, LauncherError::MissingInternalRole));
}

#[test]
fn finish_web_shutdown_propagates_signal_errors_after_cleanup() {
    let error = finish_web_shutdown(
        Err(LauncherError::WaitForWebShutdownSignal {
            source: std::io::Error::other("signal wait failed"),
        }),
        Ok(()),
    )
    .expect_err("signal errors should win");

    assert!(matches!(
        error,
        LauncherError::WaitForWebShutdownSignal { .. }
    ));
}

#[test]
fn finish_web_launch_cleanup_tolerates_shutdown_errors() {
    finish_web_launch_cleanup(Err(LauncherError::ChildExit {
        role: "web backend",
        code: Some(9),
    }));
}

#[test]
fn finish_web_shutdown_preserves_signal_errors_when_cleanup_also_fails() {
    let error = finish_web_shutdown(
        Err(LauncherError::WaitForWebShutdownSignal {
            source: std::io::Error::other("signal wait failed"),
        }),
        Err(LauncherError::ChildExit {
            role: "web backend",
            code: Some(9),
        }),
    )
    .expect_err("signal errors should still win");

    assert!(matches!(
        error,
        LauncherError::WaitForWebShutdownSignal { .. }
    ));
}

#[test]
fn finish_cli_launch_preserves_cli_errors_after_cleanup() {
    let error = finish_cli_launch(
        Err(LauncherError::RunCli {
            message: "boom".to_string(),
        }),
        Ok(()),
    )
    .expect_err("CLI errors should win");

    assert!(matches!(error, LauncherError::RunCli { .. }));
}

#[tokio::test]
async fn run_web_launcher_with_signal_cleans_up_after_entrypoint_failures() {
    let _guard = lock_acp_server_url_async().await;
    let _url_guard = test_acp_server_url_guard(None);
    let mut stack = launcher_stack::LauncherStack::direct();

    let error = run_web_launcher_with_signal(&mut stack, std::future::pending())
        .await
        .expect_err("missing backend URLs should fail");

    assert!(matches!(error, LauncherError::MissingBackendUrl));
    assert!(!stack.is_ephemeral());
}

#[tokio::test]
async fn run_web_launcher_with_signal_shuts_down_ephemeral_stacks() {
    let (base_url, handle) =
        spawn_single_response_http_server("HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n").await;
    let backend = spawn_sleep_child().await;
    let mut stack =
        launcher_stack::LauncherStack::ephemeral(backend, None, base_url, "token".to_string());

    run_web_launcher_with_signal(&mut stack, std::future::ready(Ok(())))
        .await
        .expect("ephemeral web launches should shut down after the signal");

    assert!(!stack.is_ephemeral());
    assert!(stack.auth_token().is_none());
    handle.abort();
    let _ = handle.await;
}

#[tokio::test]
async fn run_launcher_routes_web_mode_through_the_web_launcher() {
    let (base_url, handle) =
        spawn_single_response_http_server("HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n").await;
    let _guard = lock_acp_server_url_async().await;
    let _url_guard = test_acp_server_url_guard(Some(base_url.as_str()));

    let result = run_launcher(
        Path::new("/bin/true"),
        LauncherArgs {
            acp_server: None,
            web: true,
            cli_args: Vec::new(),
        },
    )
    .await;

    handle.abort();
    let _ = handle.await;
    result.expect("web launcher routing should succeed");
}

#[tokio::test]
async fn prepare_frontend_dist_skips_external_backend_launches() {
    let _guard = test_env_lock().lock().await;
    let _url_guard = test_acp_server_url_guard(Some("https://127.0.0.1:9443"));

    let frontend_dist = prepare_managed_web_frontend_dist()
        .await
        .expect("external backend launches should skip the managed frontend build");

    assert_eq!(frontend_dist, None);
}

#[tokio::test]
async fn prepare_frontend_dist_returns_workspace_dist_for_managed_web_launches() {
    let _guard = test_env_lock().lock().await;
    let _url_guard = test_acp_server_url_guard(None);
    let dist = frontend_dist_path();
    fs::create_dir_all(&dist).expect("frontend dist directory should be creatable");
    let created_assets = write_stub_frontend_bundle_assets(&dist);

    let frontend_dist = prepare_managed_web_frontend_dist()
        .await
        .expect("managed web launches should prepare the frontend dist");

    assert_eq!(frontend_dist.as_deref(), Some(dist.as_path()));

    for asset in created_assets {
        let _ = fs::remove_file(asset);
    }
}

#[tokio::test]
async fn ensure_frontend_built_reports_missing_trunk() {
    let _guard = test_env_lock().lock().await;
    let _path_guard = test_path_guard(Some(path_without_trunk()));
    let dist = unique_temp_json_path("acp-frontend-dist", "missing-trunk").with_extension("");
    fs::create_dir_all(&dist).expect("frontend dist directory should be creatable");

    let error = ensure_frontend_built(&dist)
        .await
        .expect_err("missing trunk executables should be surfaced");

    assert!(matches!(
        error,
        LauncherError::FrontendBuild { message }
            if message == "trunk not found – install it with `cargo install trunk`"
    ));
}

#[tokio::test]
async fn ensure_frontend_built_surfaces_failed_trunk_exit_codes() {
    let _guard = test_env_lock().lock().await;
    let fake_trunk_dir = write_fake_trunk_bin("exit 9");
    let _path_guard = test_path_guard(Some(path_with_fake_trunk(&fake_trunk_dir)));
    let dist = unique_temp_json_path("acp-frontend-dist", "failed-trunk").with_extension("");
    fs::create_dir_all(&dist).expect("frontend dist directory should be creatable");

    let error = ensure_frontend_built(&dist)
        .await
        .expect_err("failed trunk builds should be surfaced");

    assert!(matches!(
        error,
        LauncherError::FrontendBuild { message }
            if message == "`trunk build --release` failed with exit code Some(9)"
    ));
}

#[tokio::test]
async fn ensure_frontend_built_surfaces_other_trunk_spawn_errors() {
    let _guard = test_env_lock().lock().await;
    let fake_trunk_dir = write_fake_trunk_bin_with_permissions("exit 0", 0o644);
    let _path_guard = test_path_guard(Some(path_with_fake_trunk(&fake_trunk_dir)));
    let dist = unique_temp_json_path("acp-frontend-dist", "unexecutable-trunk").with_extension("");
    fs::create_dir_all(&dist).expect("frontend dist directory should be creatable");

    let error = ensure_frontend_built(&dist)
        .await
        .expect_err("other trunk spawn failures should be surfaced");

    assert!(matches!(
        error,
        LauncherError::FrontendBuild { message } if !message.is_empty()
    ));
}

#[tokio::test]
async fn ensure_frontend_built_accepts_successful_trunk_runs() {
    let _guard = test_env_lock().lock().await;
    let fake_trunk_dir = write_fake_trunk_bin("exit 0");
    let _path_guard = test_path_guard(Some(path_with_fake_trunk(&fake_trunk_dir)));
    let dist = unique_temp_json_path("acp-frontend-dist", "successful-trunk").with_extension("");
    fs::create_dir_all(&dist).expect("frontend dist directory should be creatable");

    ensure_frontend_built(&dist)
        .await
        .expect("successful trunk builds should be accepted");
}

#[test]
fn finish_cli_launch_preserves_cli_errors_when_cleanup_also_fails() {
    let error = finish_cli_launch(
        Err(LauncherError::RunCli {
            message: "boom".to_string(),
        }),
        Err(LauncherError::ChildExit {
            role: "web backend",
            code: Some(9),
        }),
    )
    .expect_err("CLI errors should still win");

    assert!(matches!(error, LauncherError::RunCli { .. }));
}

#[tokio::test]
async fn wait_for_web_entrypoint_succeeds_when_the_app_route_is_ready() {
    let (base_url, handle) =
        spawn_single_response_http_server("HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n").await;

    let app_url = wait_for_web_entrypoint(&base_url)
        .await
        .expect("web readiness should succeed");

    assert_eq!(app_url, format!("{base_url}/app/"));
    handle.abort();
    let _ = handle.await;
}

#[tokio::test]
async fn wait_for_web_entrypoint_reports_failures() {
    let (base_url, handle) = spawn_single_response_http_server(
        "HTTP/1.1 503 Service Unavailable\r\ncontent-length: 0\r\n\r\n",
    )
    .await;

    let error = wait_for_web_entrypoint(&base_url)
        .await
        .expect_err("503 responses should fail");

    assert!(matches!(error, LauncherError::WaitForWebEntryPoint { .. }));
    handle.abort();
    let _ = handle.await;
}

#[tokio::test]
async fn run_web_foreground_returns_ok_even_when_opening_the_browser_fails() {
    let (base_url, handle) =
        spawn_single_response_http_server("HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n").await;
    let stack = launcher_stack::LauncherStack::persistent(base_url, "token".to_string());

    run_web_foreground_with(&stack, |_| Err(std::io::Error::other("boom")))
        .await
        .expect("web foreground launch should still succeed");

    handle.abort();
    let _ = handle.await;
}

#[tokio::test]
async fn read_startup_url_rejects_empty_stdout() {
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(":")
        .stdout(Stdio::piped())
        .spawn()
        .expect("child should spawn");

    let error = read_startup_url(&mut child, "test role")
        .await
        .expect_err("empty stdout should fail");

    assert!(matches!(
        error,
        LauncherError::InvalidStartupLine { line, .. } if line == "<empty>"
    ));
}

#[tokio::test]
async fn read_startup_url_rejects_invalid_stdout() {
    let mut child = Command::new("sh")
        .arg("-c")
        .arg("printf 'ready\\n'")
        .stdout(Stdio::piped())
        .spawn()
        .expect("child should spawn");

    let error = read_startup_url(&mut child, "test role")
        .await
        .expect_err("invalid stdout should fail");

    assert!(matches!(
        error,
        LauncherError::InvalidStartupLine { line, .. } if line == "ready"
    ));
}

#[tokio::test]
async fn terminate_child_returns_for_already_exited_processes() {
    let mut child = Command::new("sh")
        .arg("-c")
        .arg("exit 0")
        .spawn()
        .expect("child should spawn");
    let _ = child.wait().await.expect("child should exit");

    terminate_child(&mut child, "test role")
        .await
        .expect("already exited child should be ignored");
}

#[test]
fn ensure_success_rejects_non_zero_exit_codes() {
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg("exit 3")
        .status()
        .expect("status should be available");

    let error = ensure_success("cli frontend", status).expect_err("non-zero exits should fail");

    assert!(matches!(
        error,
        LauncherError::ChildExit {
            role: "cli frontend",
            code: Some(3)
        }
    ));
}

#[tokio::test]
async fn run_internal_role_reports_unknown_roles() {
    let error = run_internal_role(OsString::from("unknown"), Vec::new())
        .await
        .expect_err("unknown roles should fail");

    assert!(matches!(
        error,
        LauncherError::UnknownInternalRole { role } if role == "unknown"
    ));
}

#[tokio::test]
async fn run_internal_role_wraps_cli_errors() {
    let error = run_internal_role(
        OsString::from("cli"),
        vec![OsString::from("chat"), OsString::from("--new")],
    )
    .await
    .expect_err("invalid cli invocation should fail");

    assert!(matches!(error, LauncherError::RunCli { .. }));
}

#[tokio::test]
async fn run_mock_role_validates_arguments() {
    let error = run_mock_role(vec![OsString::from("--unexpected")])
        .await
        .expect_err("unexpected args should fail");
    assert!(matches!(error, LauncherError::RunMock { .. }));

    let error = run_mock_role(vec![OsString::from("--port"), OsString::from("nope")])
        .await
        .expect_err("invalid ports should fail");
    assert!(matches!(error, LauncherError::RunMock { .. }));
}

#[tokio::test]
async fn run_mock_role_can_shutdown_cleanly() {
    run_mock_role(vec![
        OsString::from("--port"),
        OsString::from("0"),
        OsString::from("--response-delay-ms"),
        OsString::from("1"),
        OsString::from("--exit-after-ms"),
        OsString::from("50"),
    ])
    .await
    .expect("mock role should stop cleanly");
}

#[tokio::test]
async fn run_mock_role_can_start_without_a_test_shutdown() {
    let local_set = tokio::task::LocalSet::new();
    let result = tokio::time::timeout(
        Duration::from_millis(50),
        local_set.run_until(run_mock_role(vec![
            OsString::from("--port"),
            OsString::from("0"),
            OsString::from("--response-delay-ms"),
            OsString::from("1"),
        ])),
    )
    .await;

    assert!(result.is_err(), "mock role should keep running");
}

#[tokio::test]
async fn run_backend_role_validates_arguments() {
    let error = run_backend_role(vec![OsString::from("--unexpected")])
        .await
        .expect_err("unexpected args should fail");
    assert!(matches!(error, LauncherError::RunBackend { .. }));

    let error = run_backend_role(Vec::new())
        .await
        .expect_err("missing ACP server addresses should fail");
    assert!(matches!(error, LauncherError::RunBackend { .. }));

    let error = run_backend_role(vec![
        OsString::from("--session-cap"),
        OsString::from("nope"),
    ])
    .await
    .expect_err("invalid session caps should fail");
    assert!(matches!(error, LauncherError::RunBackend { .. }));
}

#[tokio::test]
async fn run_backend_role_can_shutdown_cleanly() {
    run_backend_role(vec![
        OsString::from("--port"),
        OsString::from("0"),
        OsString::from("--acp-server"),
        OsString::from("127.0.0.1:9"),
        OsString::from("--exit-after-ms"),
        OsString::from("50"),
    ])
    .await
    .expect("backend role should stop cleanly");
}

#[tokio::test]
async fn run_backend_role_can_start_without_a_test_shutdown() {
    let handle = tokio::spawn(run_backend_role(vec![
        OsString::from("--port"),
        OsString::from("0"),
        OsString::from("--acp-server"),
        OsString::from("127.0.0.1:9"),
    ]));

    tokio::time::sleep(Duration::from_millis(50)).await;
    handle.abort();
    let _ = handle.await;
}

fn lock_acp_server_url() -> MutexGuard<'static, ()> {
    test_env_lock().blocking_lock()
}

async fn lock_acp_server_url_async() -> MutexGuard<'static, ()> {
    test_env_lock().lock().await
}

async fn spawn_single_response_http_server(
    response: &'static str,
) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("HTTP listener should bind");
    let address = listener
        .local_addr()
        .expect("HTTP listener should expose its address");
    let handle = tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request).await;
            let _ = stream.write_all(response.as_bytes()).await;
        }
    });

    (format!("http://{address}"), handle)
}

async fn spawn_sleep_child() -> tokio::process::Child {
    Command::new("sh")
        .arg("-c")
        .arg("sleep 30")
        .spawn()
        .expect("sleep child should spawn")
}

fn write_stub_frontend_bundle_assets(dist: &Path) -> Vec<PathBuf> {
    let tag = uuid::Uuid::new_v4();
    let javascript = dist.join(frontend_bundle_file_name(
        &tag.to_string(),
        FrontendBundleAsset::JavaScript,
    ));
    let wasm = dist.join(frontend_bundle_file_name(
        &tag.to_string(),
        FrontendBundleAsset::Wasm,
    ));
    fs::write(&javascript, "export default async function init() {}\n")
        .expect("stub javascript bundle should write");
    fs::write(&wasm, b"\x00asm\x01\x00\x00\x00").expect("stub wasm bundle should write");
    vec![javascript, wasm]
}

async fn prepare_managed_web_frontend_dist() -> Result<Option<PathBuf>> {
    prepare_frontend_dist(
        &LauncherArgs {
            acp_server: None,
            web: true,
            cli_args: Vec::new(),
        },
        true,
        false,
    )
    .await
}

fn write_fake_trunk_bin(command: &str) -> PathBuf {
    write_fake_trunk_bin_with_permissions(command, 0o755)
}

fn write_fake_trunk_bin_with_permissions(command: &str, mode: u32) -> PathBuf {
    let dir = unique_temp_json_path("acp-trunk-bin", "frontend-build").with_extension("");
    fs::create_dir_all(&dir).expect("fake trunk bin dir should be creatable");
    let trunk = dir.join("trunk");
    fs::write(&trunk, format!("#!/bin/sh\n{command}\n")).expect("fake trunk should write");
    let mut permissions = fs::metadata(&trunk)
        .expect("fake trunk metadata should load")
        .permissions();
    permissions.set_mode(mode);
    fs::set_permissions(&trunk, permissions).expect("fake trunk should become executable");
    dir
}

fn path_with_fake_trunk(front: &Path) -> OsString {
    std::env::join_paths([front, Path::new("/bin"), Path::new("/usr/bin")])
        .expect("PATH entries should be joinable")
}

fn path_without_trunk() -> OsString {
    std::env::join_paths([Path::new("/bin"), Path::new("/usr/bin")])
        .expect("PATH entries should be joinable")
}
