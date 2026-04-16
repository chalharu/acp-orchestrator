use super::*;
use std::{
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use reqwest::Client;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::{net::TcpListener, process::Command, time::timeout};

#[test]
fn init_tracing_can_be_called_more_than_once() {
    init_tracing();
    init_tracing();
}

#[test]
fn unique_temp_json_path_uses_the_expected_shape() {
    let path = unique_temp_json_path("acp", "support");

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("temp path should have a UTF-8 file name");
    assert!(file_name.starts_with("acp-support-"));
    assert!(file_name.ends_with(".json"));
}

#[tokio::test]
async fn shutdown_signal_resolves_when_a_deadline_is_set() {
    timeout(Duration::from_millis(100), shutdown_signal(Some(5)))
        .await
        .expect("shutdown signal should resolve");
}

#[tokio::test]
async fn shutdown_signal_stays_pending_without_a_deadline() {
    let result = timeout(Duration::from_millis(20), shutdown_signal(None)).await;
    assert!(result.is_err(), "pending shutdown should time out");
}

#[tokio::test]
async fn read_startup_url_reads_the_expected_prefix() {
    let mut child = Command::new("sh")
        .arg("-c")
        .arg("printf 'service listening on http://127.0.0.1:4321\\n'")
        .stdout(Stdio::piped())
        .spawn()
        .expect("child should spawn");

    let url = read_startup_url(&mut child, "service listening on ")
        .await
        .expect("startup line should parse");

    assert_eq!(url, "http://127.0.0.1:4321");
}

#[tokio::test]
async fn read_startup_url_requires_a_stdout_pipe() {
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(":")
        .stdout(Stdio::null())
        .spawn()
        .expect("child should spawn");

    let error = read_startup_url(&mut child, "service listening on ")
        .await
        .expect_err("missing stdout should fail");

    assert!(error.to_string().contains("missing child stdout"));
}

#[tokio::test]
async fn read_startup_url_rejects_unexpected_prefixes() {
    let mut child = Command::new("sh")
        .arg("-c")
        .arg("printf 'unexpected\\n'")
        .stdout(Stdio::piped())
        .spawn()
        .expect("child should spawn");

    let error = read_startup_url(&mut child, "service listening on ")
        .await
        .expect_err("unexpected startup lines should fail");

    assert!(error.to_string().contains("unexpected startup line"));
}

#[tokio::test]
async fn bind_listener_reports_successful_binding() {
    let listener = bind_listener("127.0.0.1", 0, "test service")
        .await
        .expect("listener should bind");
    let address = listener
        .local_addr()
        .expect("listener should expose its address");

    assert!(address.port() > 0);
}

#[tokio::test]
async fn bind_listener_reports_bind_failures() {
    let occupied = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let port = occupied
        .local_addr()
        .expect("listener should expose its address")
        .port();

    let error = bind_listener("127.0.0.1", port, "test service")
        .await
        .expect_err("occupied ports should fail");

    assert!(
        matches!(error, ListenerSetupError::Bind { port: bound_port, .. } if bound_port == port)
    );
}

#[tokio::test]
async fn listener_endpoint_formats_the_bound_address() {
    let listener = bind_listener("127.0.0.1", 0, "test service")
        .await
        .expect("listener should bind");

    let endpoint =
        listener_endpoint(&listener, "test service", "http://").expect("endpoint should format");

    assert!(endpoint.starts_with("http://127.0.0.1:"));
}

#[test]
fn build_http_client_for_loopback_urls_succeeds() {
    build_http_client_for_url("http://127.0.0.1:8080", Some(Duration::from_secs(1)))
        .expect("loopback clients should build");
    build_http_client_for_url("https://127.0.0.1:8443", Some(Duration::from_secs(1)))
        .expect("loopback https clients should build");
}

#[test]
fn build_http_client_for_remote_urls_succeeds() {
    build_http_client_for_url("https://example.com", None).expect("remote clients should build");
}

#[test]
fn proxy_bypass_is_enabled_for_loopback_urls() {
    assert!(should_bypass_proxy_for_url("http://127.0.0.1:8080"));
    assert!(should_bypass_proxy_for_url("http://localhost:8080"));
    assert!(should_bypass_proxy_for_url("http://[::1]:8080"));
}

#[test]
fn proxy_bypass_is_disabled_for_remote_and_invalid_urls() {
    assert!(!should_bypass_proxy_for_url("https://example.com"));
    assert!(!should_bypass_proxy_for_url("mailto:test@example.com"));
    assert!(!should_bypass_proxy_for_url("not-a-url"));
}

#[test]
fn invalid_cert_trust_is_limited_to_literal_loopback_https() {
    assert!(should_trust_invalid_loopback_cert_for_url(
        "https://127.0.0.1:8443"
    ));
    assert!(!should_trust_invalid_loopback_cert_for_url(
        "https://localhost:8443"
    ));
    assert!(!should_trust_invalid_loopback_cert_for_url(
        "http://127.0.0.1:8080"
    ));
    assert!(!should_trust_invalid_loopback_cert_for_url(
        "https://example.com"
    ));
}

#[tokio::test]
async fn wait_for_tcp_connect_succeeds_when_the_listener_is_ready() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let address = listener
        .local_addr()
        .expect("listener should expose its address");
    let handle = tokio::spawn(async move {
        listener
            .accept()
            .await
            .expect("probe connection should reach the listener");
    });

    wait_for_tcp_connect(&address.to_string(), 10, Duration::from_millis(5))
        .await
        .expect("TCP wait should succeed");
    handle
        .await
        .expect("listener task should complete after the probe connection");
}

#[tokio::test]
async fn wait_for_tcp_connect_reports_failures_after_exhausting_retries() {
    let error = wait_for_tcp_connect("127.0.0.1:9", 2, Duration::from_millis(1))
        .await
        .expect_err("unreachable TCP services should fail");

    assert!(
        error
            .to_string()
            .contains("TCP service did not accept connections")
    );
}

#[tokio::test]
async fn wait_for_health_succeeds_when_the_endpoint_is_ready() {
    let client = Client::builder().build().expect("test client should build");
    let (base_url, handle) = spawn_health_server().await;

    wait_for_health(&client, &base_url, 10, Duration::from_millis(5))
        .await
        .expect("health check should succeed");
    handle.abort();
    let _ = handle.await;
}

#[tokio::test]
async fn wait_for_health_reports_failures_after_exhausting_retries() {
    let client = Client::builder().build().expect("test client should build");

    let error = wait_for_health(&client, "http://127.0.0.1:9", 2, Duration::from_millis(1))
        .await
        .expect_err("unreachable health endpoints should fail");

    assert!(error.to_string().contains("health check did not succeed"));
}

#[tokio::test]
async fn wait_for_http_success_succeeds_when_the_endpoint_is_ready() {
    let client = Client::builder().build().expect("test client should build");
    let (base_url, handle) = spawn_health_server().await;
    let app_url = format!("{base_url}/app/");

    wait_for_http_success(
        &client,
        &app_url,
        10,
        Duration::from_millis(5),
        "browser entrypoint",
    )
    .await
    .expect("HTTP success wait should succeed");
    handle.abort();
    let _ = handle.await;
}

#[tokio::test]
async fn wait_for_http_success_reports_failures_after_exhausting_retries() {
    let client = Client::builder().build().expect("test client should build");

    let error = wait_for_http_success(
        &client,
        "http://127.0.0.1:9/app/",
        2,
        Duration::from_millis(1),
        "browser entrypoint",
    )
    .await
    .expect_err("unreachable endpoints should fail");

    assert!(
        error
            .to_string()
            .contains("browser entrypoint did not succeed")
    );
}

#[tokio::test]
async fn run_service_with_readiness_calls_the_ready_callback_before_waiting_for_shutdown() {
    let ready_called = Arc::new(AtomicBool::new(false));
    let ready_called_for_assert = ready_called.clone();

    run_service_with_readiness(
        async { Ok::<(), io::Error>(()) },
        async {
            tokio::time::sleep(Duration::from_millis(10)).await;
            Ok::<(), io::Error>(())
        },
        move || {
            ready_called.store(true, Ordering::SeqCst);
        },
    )
    .await
    .expect("service should run after readiness succeeds");

    assert!(ready_called_for_assert.load(Ordering::SeqCst));
}

#[tokio::test]
async fn run_service_with_readiness_surfaces_service_failures_before_ready() {
    let error = run_service_with_readiness(
        std::future::pending::<std::result::Result<(), io::Error>>(),
        std::future::ready(Err::<(), _>(io::Error::other("boom"))),
        Default::default,
    )
    .await
    .expect_err("service errors should win when they happen first");

    assert!(matches!(error, ServiceReadinessError::Run(_)));
}

#[tokio::test]
async fn run_service_with_readiness_surfaces_readiness_failures() {
    let error = run_service_with_readiness(
        std::future::ready(Err::<(), _>(io::Error::other("not ready"))),
        std::future::pending::<std::result::Result<(), io::Error>>(),
        Default::default,
    )
    .await
    .expect_err("readiness failures should be surfaced");

    assert!(matches!(error, ServiceReadinessError::Ready(_)));
}

async fn spawn_health_server() -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("health listener should bind");
    let address = listener
        .local_addr()
        .expect("health listener should expose its address");
    let handle = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("health server should accept one request");
        let mut request = [0u8; 1024];
        let _ = stream.read(&mut request).await;
        let _ = stream
            .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n")
            .await;
    });

    (format!("http://{address}"), handle)
}
