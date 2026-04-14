use std::{
    error::Error as StdError,
    future::{Future, pending},
    io,
    net::IpAddr,
    path::PathBuf,
    pin::Pin,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use clap::Args;
use reqwest::Client;
use snafu::prelude::*;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    net::{TcpListener, TcpStream},
    process::Child,
};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

pub type BoxError = Box<dyn StdError + Send + Sync>;
pub type ShutdownSignal = Pin<Box<dyn Future<Output = ()> + Send>>;
pub type SupportResult<T, E> = std::result::Result<T, E>;

#[derive(Debug)]
pub enum ServiceReadinessError<E> {
    Ready(E),
    Run(io::Error),
}

#[derive(Debug, Args, Clone)]
pub struct RuntimeListenArgs {
    #[arg(long, default_value = "127.0.0.1")]
    pub host: String,
    #[arg(long, hide = true)]
    pub exit_after_ms: Option<u64>,
}

#[derive(Debug, Snafu)]
pub enum ListenerSetupError {
    #[snafu(display("binding the {service_name} on {host}:{port} failed"))]
    Bind {
        source: io::Error,
        service_name: &'static str,
        host: String,
        port: u16,
    },

    #[snafu(display("reading the bound {service_name} address failed"))]
    ReadBoundAddress {
        source: io::Error,
        service_name: &'static str,
    },
}

pub fn init_tracing() {
    let _ = tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .without_time()
                .with_writer(std::io::stderr),
        )
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .try_init();
}

pub async fn bind_listener(
    host: &str,
    port: u16,
    service_name: &'static str,
) -> Result<TcpListener, ListenerSetupError> {
    init_tracing();

    TcpListener::bind((host, port)).await.context(BindSnafu {
        service_name,
        host: host.to_string(),
        port,
    })
}

pub fn listener_endpoint(
    listener: &TcpListener,
    service_name: &'static str,
    startup_prefix: &'static str,
) -> Result<String, ListenerSetupError> {
    let address = listener
        .local_addr()
        .context(ReadBoundAddressSnafu { service_name })?;
    Ok(format!("{startup_prefix}{address}"))
}

pub fn print_startup_line(startup_label: &'static str, endpoint: &str) {
    println!("{startup_label} listening on {endpoint}");
}

pub fn build_http_client_for_url(
    base_url: &str,
    timeout: Option<Duration>,
) -> Result<Client, reqwest::Error> {
    let mut builder = Client::builder();
    if should_bypass_proxy_for_url(base_url) {
        builder = builder.no_proxy();
    }
    if let Some(timeout) = timeout {
        builder = builder.timeout(timeout);
    }
    builder.build()
}

fn should_bypass_proxy_for_url(base_url: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(base_url) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host.trim_matches(|character| character == '[' || character == ']');

    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

pub async fn run_service_with_readiness<E, Ready, Serve, OnReady>(
    ready: Ready,
    serve: Serve,
    on_ready: OnReady,
) -> SupportResult<(), ServiceReadinessError<E>>
where
    Ready: Future<Output = SupportResult<(), E>>,
    Serve: Future<Output = io::Result<()>>,
    OnReady: FnOnce(),
{
    tokio::pin!(ready);
    tokio::pin!(serve);

    tokio::select! {
        result = &mut ready => {
            result.map_err(ServiceReadinessError::Ready)?;
            on_ready();
            serve.await.map_err(ServiceReadinessError::Run)
        }
        result = &mut serve => result.map_err(ServiceReadinessError::Run),
    }
}

pub fn shutdown_signal(exit_after_ms: Option<u64>) -> ShutdownSignal {
    if let Some(exit_after_ms) = exit_after_ms {
        Box::pin(tokio::time::sleep(Duration::from_millis(exit_after_ms)))
    } else {
        Box::pin(pending())
    }
}

pub async fn read_startup_url(child: &mut Child, prefix: &str) -> Result<String, BoxError> {
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("missing child stdout"))?;
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader.read_line(&mut line).await?;
    Ok(line
        .trim()
        .strip_prefix(prefix)
        .ok_or_else(|| io::Error::other(format!("unexpected startup line: {}", line.trim())))?
        .to_string())
}

pub async fn wait_for_health(
    client: &Client,
    base_url: &str,
    attempts: usize,
    delay: Duration,
) -> Result<(), BoxError> {
    let health_url = format!("{base_url}/healthz");
    for _ in 0..attempts {
        if let Ok(response) = client.get(&health_url).send().await
            && response.status().is_success()
        {
            return Ok(());
        }
        tokio::time::sleep(delay).await;
    }

    Err(io::Error::other(format!("health check did not succeed for {health_url}")).into())
}

pub fn unique_temp_json_path(prefix: &str, label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after the epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("{prefix}-{label}-{nanos}.json"))
}

pub async fn wait_for_tcp_connect(
    address: &str,
    attempts: usize,
    delay: Duration,
) -> Result<(), BoxError> {
    for _ in 0..attempts {
        if let Ok(stream) = TcpStream::connect(address).await {
            drop(stream);
            return Ok(());
        }
        tokio::time::sleep(delay).await;
    }

    Err(io::Error::other(format!(
        "TCP service did not accept connections at {address}"
    ))
    .into())
}

#[cfg(test)]
mod tests {
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

        let endpoint = listener_endpoint(&listener, "test service", "http://")
            .expect("endpoint should format");

        assert!(endpoint.starts_with("http://127.0.0.1:"));
    }

    #[test]
    fn build_http_client_for_loopback_urls_succeeds() {
        build_http_client_for_url("http://127.0.0.1:8080", Some(Duration::from_secs(1)))
            .expect("loopback clients should build");
    }

    #[test]
    fn build_http_client_for_remote_urls_succeeds() {
        build_http_client_for_url("https://example.com", None)
            .expect("remote clients should build");
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
}
