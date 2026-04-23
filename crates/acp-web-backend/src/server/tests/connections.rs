use super::*;

#[tokio::test]
async fn serving_with_shutdown_handles_successful_connections() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("test listener should bind");
    let address = listener
        .local_addr()
        .expect("test listener should expose its address");
    let base_url = format!("https://{address}");
    let client = build_http_client_for_url(&base_url, Some(Duration::from_secs(1)))
        .expect("loopback clients should build");
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        serve_with_shutdown(listener, test_state(), async move {
            let _ = shutdown_rx.await;
        })
        .await
    });

    let response = client
        .get(format!("{base_url}/healthz"))
        .send()
        .await
        .expect("health requests should reach the server");
    response
        .error_for_status()
        .expect("health requests should succeed")
        .bytes()
        .await
        .expect("health responses should be readable");

    drop(client);
    tokio::task::yield_now().await;
    shutdown_tx
        .send(())
        .expect("shutdown signals should reach the server");

    timeout(Duration::from_secs(1), server)
        .await
        .expect("the server should stop promptly")
        .expect("the server task should join")
        .expect("serving should shut down cleanly");
}

#[tokio::test]
async fn aborted_connection_tasks_are_logged_without_panicking() {
    let mut connections = tokio::task::JoinSet::new();
    connections.spawn(async {
        panic!("boom");
    });

    let next = connections.join_next().await;
    log_connection_task_join_result(next);

    assert!(connections.is_empty());
}

#[test]
fn connection_results_are_logged_without_panicking() {
    log_connection_result(Ok::<(), std::io::Error>(()));
    log_connection_result(Err(std::io::Error::other("boom")));
}

#[tokio::test]
async fn successful_accepts_reset_transient_failure_counts() {
    let (address, accepted_stream, client) = accept_test_stream().await;
    let mut failures = 3usize;
    let mut connections = tokio::task::JoinSet::new();
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let acceptor = test_tls_acceptor(address);
    let router = test_router();
    let shutdown = std::future::pending::<()>();
    tokio::pin!(shutdown);

    let action = handle_accept_result(
        Ok((accepted_stream, address)),
        &mut failures,
        AcceptContext {
            connections: &mut connections,
            tls_acceptor: &acceptor,
            app: &router,
            shutdown_rx: &shutdown_rx,
            shutdown_tx: &shutdown_tx,
        },
        shutdown.as_mut(),
    )
    .await
    .expect("successful accepts should continue serving");

    assert_eq!(action, AcceptLoopAction::Continue);
    assert_eq!(failures, 0);

    drop(client);
    timeout(Duration::from_secs(1), connections.join_next())
        .await
        .expect("the spawned TLS task should observe the failed handshake");
}

#[tokio::test]
async fn transient_accept_failures_retry_after_backoff() {
    let mut failures = 0usize;
    let mut connections = tokio::task::JoinSet::new();
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let acceptor = loopback_test_acceptor();
    let router = test_router();
    let shutdown = std::future::pending::<()>();
    tokio::pin!(shutdown);

    let action = handle_accept_result(
        Err(std::io::Error::from(std::io::ErrorKind::WouldBlock)),
        &mut failures,
        AcceptContext {
            connections: &mut connections,
            tls_acceptor: &acceptor,
            app: &router,
            shutdown_rx: &shutdown_rx,
            shutdown_tx: &shutdown_tx,
        },
        shutdown.as_mut(),
    )
    .await
    .expect("retryable accept errors should not fail serving");

    assert_eq!(action, AcceptLoopAction::Continue);
    assert_eq!(failures, 1);
}

#[tokio::test]
async fn transient_accept_failures_break_when_shutdown_arrives() {
    let mut failures = 0usize;
    let mut connections = tokio::task::JoinSet::new();
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let acceptor = loopback_test_acceptor();
    let router = test_router();
    let shutdown = std::future::ready(());
    tokio::pin!(shutdown);

    let action = handle_accept_result(
        Err(std::io::Error::from(std::io::ErrorKind::WouldBlock)),
        &mut failures,
        AcceptContext {
            connections: &mut connections,
            tls_acceptor: &acceptor,
            app: &router,
            shutdown_rx: &shutdown_rx,
            shutdown_tx: &shutdown_tx,
        },
        shutdown.as_mut(),
    )
    .await
    .expect("shutdown during backoff should stop serving cleanly");

    assert_eq!(action, AcceptLoopAction::Break);
    assert_eq!(failures, 1);
}

#[tokio::test]
async fn finish_accept_loop_if_requested_returns_false_for_continue() {
    let mut connections = tokio::task::JoinSet::new();
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    assert!(
        !finish_accept_loop_if_requested(
            AcceptLoopAction::Continue,
            &shutdown_tx,
            &mut connections
        )
        .await
    );
    assert!(!*shutdown_rx.borrow());
    assert!(connections.is_empty());
}

#[tokio::test]
async fn finish_accept_loop_if_requested_drains_connections_for_break() {
    let mut connections = tokio::task::JoinSet::new();
    connections.spawn(async {});
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    assert!(
        finish_accept_loop_if_requested(AcceptLoopAction::Break, &shutdown_tx, &mut connections)
            .await
    );
    assert!(*shutdown_rx.borrow());
    assert!(connections.is_empty());
}

#[tokio::test]
async fn too_many_transient_accept_failures_stop_serving() {
    let mut failures = MAX_CONSECUTIVE_TRANSIENT_ACCEPT_ERRORS;
    let mut connections = tokio::task::JoinSet::new();
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let acceptor = loopback_test_acceptor();
    let router = test_router();
    let shutdown = std::future::pending::<()>();
    tokio::pin!(shutdown);

    let error = handle_accept_result(
        Err(std::io::Error::from(std::io::ErrorKind::WouldBlock)),
        &mut failures,
        AcceptContext {
            connections: &mut connections,
            tls_acceptor: &acceptor,
            app: &router,
            shutdown_rx: &shutdown_rx,
            shutdown_tx: &shutdown_tx,
        },
        shutdown.as_mut(),
    )
    .await
    .expect_err("too many retryable failures should stop serving");

    assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
    assert_eq!(failures, MAX_CONSECUTIVE_TRANSIENT_ACCEPT_ERRORS + 1);
}

#[tokio::test]
async fn fatal_accept_failures_stop_serving_immediately() {
    let mut failures = 0usize;
    let mut connections = tokio::task::JoinSet::new();
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let acceptor = loopback_test_acceptor();
    let router = test_router();
    let shutdown = std::future::pending::<()>();
    tokio::pin!(shutdown);

    let error = handle_accept_result(
        Err(std::io::Error::other("boom")),
        &mut failures,
        AcceptContext {
            connections: &mut connections,
            tls_acceptor: &acceptor,
            app: &router,
            shutdown_rx: &shutdown_rx,
            shutdown_tx: &shutdown_tx,
        },
        shutdown.as_mut(),
    )
    .await
    .expect_err("fatal accept errors should stop serving");

    assert_eq!(error.kind(), std::io::ErrorKind::Other);
    assert_eq!(failures, 0);
}

#[tokio::test]
async fn spawned_connection_tasks_handle_failed_tls_handshakes() {
    let (address, stream, client) = accept_test_stream().await;
    let mut connections = tokio::task::JoinSet::new();
    let (_, shutdown_rx) = tokio::sync::watch::channel(false);

    spawn_test_connection_task(&mut connections, address, shutdown_rx, stream);

    drop(client);
    timeout(Duration::from_secs(1), connections.join_next())
        .await
        .expect("failed TLS handshakes should finish promptly");
}

#[tokio::test]
async fn spawned_connection_tasks_honor_shutdown_signals() {
    let (address, stream, _client, request) = prepare_shutdown_test_connection().await;
    let mut connections = tokio::task::JoinSet::new();
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    spawn_test_connection_task(&mut connections, address, shutdown_rx, stream);

    request
        .await
        .expect("the client request should finish successfully");
    shutdown_tx
        .send(true)
        .expect("shutdown signals should be broadcast");

    timeout(Duration::from_secs(1), connections.join_next())
        .await
        .expect("shutdown should drain active connections");
}

#[tokio::test]
async fn draining_empty_connection_tasks_returns_immediately() {
    let mut connections = tokio::task::JoinSet::new();

    timeout(
        Duration::from_millis(50),
        drain_connection_tasks(&mut connections),
    )
    .await
    .expect("empty connection sets should not wait for the shutdown grace period");
}

#[tokio::test]
async fn draining_pending_connection_tasks_aborts_after_the_grace_period() {
    let mut connections = tokio::task::JoinSet::new();
    connections.spawn(std::future::pending::<()>());

    timeout(
        Duration::from_secs(2),
        drain_connection_tasks(&mut connections),
    )
    .await
    .expect("pending connections should be aborted after the shutdown grace period");

    assert!(connections.is_empty());
}

#[tokio::test]
async fn draining_connection_tasks_tolerates_aborted_join_handles() {
    let mut connections = tokio::task::JoinSet::new();
    connections.spawn(async {
        panic!("boom");
    });

    timeout(
        Duration::from_secs(1),
        drain_connection_tasks(&mut connections),
    )
    .await
    .expect("aborted connection tasks should still drain promptly");

    assert!(connections.is_empty());
}

#[tokio::test]
async fn graceful_connection_shutdown_returns_after_success() {
    let future = std::future::ready(Ok::<(), std::io::Error>(()));
    tokio::pin!(future);

    finish_connection_after_shutdown(future.as_mut()).await;
}

#[tokio::test]
async fn graceful_connection_shutdown_handles_connection_errors() {
    let future = std::future::ready(Err::<(), _>(std::io::Error::other("boom")));
    tokio::pin!(future);

    finish_connection_after_shutdown(future.as_mut()).await;
}

#[tokio::test]
async fn graceful_connection_shutdown_times_out_pending_connections() {
    let future = std::future::pending::<std::io::Result<()>>();
    tokio::pin!(future);

    timeout(
        Duration::from_secs(1),
        finish_connection_after_shutdown(future.as_mut()),
    )
    .await
    .expect("pending connections should stop after the graceful shutdown deadline");
}

#[test]
fn loopback_tls_acceptor_supports_additional_loopback_addresses() {
    let address = "127.0.0.2:8443"
        .parse()
        .expect("loopback socket addresses should parse");

    build_loopback_tls_acceptor(address).expect("loopback certificates should build");
}

#[test]
fn transient_accept_errors_cover_standard_retryable_kinds() {
    for kind in [
        std::io::ErrorKind::ConnectionAborted,
        std::io::ErrorKind::Interrupted,
        std::io::ErrorKind::TimedOut,
        std::io::ErrorKind::WouldBlock,
    ] {
        assert!(accept_error_is_transient(&std::io::Error::from(kind)));
    }
}

#[cfg(unix)]
#[test]
fn transient_accept_errors_cover_retryable_errno_values() {
    for errno in [
        libc::ECONNABORTED,
        libc::EINTR,
        libc::EMFILE,
        libc::ENFILE,
        libc::ENOBUFS,
        libc::ENOMEM,
    ] {
        assert!(accept_error_is_transient(
            &std::io::Error::from_raw_os_error(errno)
        ));
    }
}

#[test]
fn transient_accept_errors_reject_fatal_errors() {
    assert!(!accept_error_is_transient(&std::io::Error::other("boom")));
}

#[test]
fn session_store_errors_map_to_matching_http_categories() {
    let cases = [
        (
            SessionStoreError::NotFound,
            StatusCode::NOT_FOUND,
            "session not found",
        ),
        (
            SessionStoreError::Forbidden,
            StatusCode::FORBIDDEN,
            "session owner mismatch",
        ),
        (
            SessionStoreError::Closed,
            StatusCode::CONFLICT,
            "session already closed",
        ),
        (
            SessionStoreError::EmptyPrompt,
            StatusCode::BAD_REQUEST,
            "prompt must not be empty",
        ),
        (
            SessionStoreError::PermissionNotFound,
            StatusCode::NOT_FOUND,
            "permission request not found",
        ),
        (
            SessionStoreError::SessionCapReached,
            StatusCode::TOO_MANY_REQUESTS,
            "session cap reached for principal",
        ),
    ];

    for (source, expected_status, expected_message) in cases {
        let error: AppError = source.into();

        assert_eq!(error.status_code(), expected_status);
        assert_eq!(error.message(), expected_message);
    }
}
