use std::{io, net::IpAddr, time::Duration};

use reqwest::Client;

use acp_app_support_errors::BoxError;

pub fn build_http_client_for_url(
    base_url: &str,
    timeout: Option<Duration>,
) -> Result<Client, reqwest::Error> {
    let mut builder = Client::builder();
    if should_bypass_proxy_for_url(base_url) {
        builder = builder.no_proxy();
    }
    if should_trust_invalid_loopback_cert_for_url(base_url) {
        // Keep the self-signed loopback exception scoped to the original URL instead of
        // allowing relaxed certificate validation to carry across redirects.
        builder = builder
            .danger_accept_invalid_certs(true)
            .redirect(reqwest::redirect::Policy::none());
    }
    if let Some(timeout) = timeout {
        builder = builder.timeout(timeout);
    }
    builder.build()
}

fn should_bypass_proxy_for_url(base_url: &str) -> bool {
    parse_base_url(base_url).is_some_and(|url| {
        let Some(host) = url.host_str() else {
            return false;
        };
        let host = host.trim_matches(|character| character == '[' || character == ']');

        host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<IpAddr>()
                .is_ok_and(|address| address.is_loopback())
    })
}

fn should_trust_invalid_loopback_cert_for_url(base_url: &str) -> bool {
    parse_base_url(base_url).is_some_and(|url| {
        let Some(host) = url.host_str() else {
            return false;
        };
        let host = host.trim_matches(|character| character == '[' || character == ']');

        // Keep invalid-cert trust narrower than proxy bypass and only allow literal loopback IPs.
        url.scheme().eq_ignore_ascii_case("https")
            && host
                .parse::<IpAddr>()
                .is_ok_and(|address| address.is_loopback())
    })
}

fn parse_base_url(base_url: &str) -> Option<reqwest::Url> {
    reqwest::Url::parse(base_url).ok()
}

pub async fn wait_for_http_success(
    client: &Client,
    url: &str,
    attempts: usize,
    delay: Duration,
    probe_name: &str,
) -> Result<(), BoxError> {
    let mut last_failure = None;
    for attempt in 0..attempts {
        match client.get(url).send().await {
            Ok(response) if response.status().is_success() => return Ok(()),
            Ok(response) => {
                last_failure = Some(format!("unexpected status {}", response.status()));
            }
            Err(error) => {
                last_failure = Some(error.to_string());
            }
        }
        if attempt + 1 < attempts {
            tokio::time::sleep(delay).await;
        }
    }

    let detail = last_failure
        .map(|failure| format!(": {failure}"))
        .unwrap_or_default();
    Err(io::Error::other(format!("{probe_name} did not succeed for {url}{detail}")).into())
}

pub async fn wait_for_health(
    client: &Client,
    base_url: &str,
    attempts: usize,
    delay: Duration,
) -> Result<(), BoxError> {
    let health_url = format!("{base_url}/healthz");
    wait_for_http_success(client, &health_url, attempts, delay, "health check").await
}

pub async fn wait_for_tcp_connect(
    address: &str,
    attempts: usize,
    delay: Duration,
) -> Result<(), BoxError> {
    for _ in 0..attempts {
        if let Ok(stream) = tokio::net::TcpStream::connect(address).await {
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
    use std::time::Duration;

    use reqwest::Client;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    use super::{
        build_http_client_for_url, should_bypass_proxy_for_url,
        should_trust_invalid_loopback_cert_for_url, wait_for_health, wait_for_http_success,
        wait_for_tcp_connect,
    };

    #[test]
    fn build_http_client_for_loopback_urls_succeeds() {
        build_http_client_for_url("http://127.0.0.1:8080", Some(Duration::from_secs(1)))
            .expect("loopback clients should build");
        build_http_client_for_url("https://127.0.0.1:8443", Some(Duration::from_secs(1)))
            .expect("loopback https clients should build");
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

    #[test]
    fn invalid_cert_trust_is_disabled_for_hostless_urls() {
        assert!(!should_trust_invalid_loopback_cert_for_url(
            "file:///tmp/acp.sock"
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
