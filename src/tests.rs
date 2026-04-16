use super::*;
use std::{process::Stdio, time::Duration};
use tokio::process::Command;

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
fn split_launcher_args_extracts_the_acp_server_override() {
    let args = vec![
        OsString::from("acp"),
        OsString::from("--acp-server"),
        OsString::from("127.0.0.1:8090"),
    ];

    assert_eq!(
        split_launcher_args(&args).expect("ACP server overrides should parse"),
        LauncherArgs {
            acp_server: Some(OsString::from("127.0.0.1:8090")),
            web: false,
            cli_args: vec![OsString::from("chat"), OsString::from("--new")],
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
fn split_launcher_args_extracts_the_equals_form_acp_server_override() {
    let args = vec![
        OsString::from("acp"),
        OsString::from("--acp-server=127.0.0.1:8090"),
    ];

    assert_eq!(
        split_launcher_args(&args).expect("ACP server overrides should parse"),
        LauncherArgs {
            acp_server: Some(OsString::from("127.0.0.1:8090")),
            web: false,
            cli_args: vec![OsString::from("chat"), OsString::from("--new")],
        }
    );
}

#[test]
fn split_launcher_args_rejects_an_empty_equals_form_acp_server_override() {
    let args = vec![OsString::from("acp"), OsString::from("--acp-server=")];

    let error = split_launcher_args(&args).expect_err("empty ACP server values should fail");

    assert!(matches!(error, LauncherError::MissingAcpServer));
}

#[test]
fn split_launcher_args_keeps_non_launcher_args_for_the_cli() {
    let args = vec![
        OsString::from("acp"),
        OsString::from("--acp-server"),
        OsString::from("127.0.0.1:8090"),
        OsString::from("chat"),
        OsString::from("--new"),
    ];

    assert_eq!(
        split_launcher_args(&args).expect("launcher args should parse"),
        LauncherArgs {
            acp_server: Some(OsString::from("127.0.0.1:8090")),
            web: false,
            cli_args: vec![OsString::from("chat"), OsString::from("--new")],
        }
    );
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
