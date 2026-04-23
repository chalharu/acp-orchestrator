use super::*;

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
