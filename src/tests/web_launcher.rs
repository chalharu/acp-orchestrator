use super::*;

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
