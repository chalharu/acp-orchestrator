use std::{
    error::Error as StdError,
    future::{Future, pending},
    io,
    path::PathBuf,
    pin::Pin,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[cfg(feature = "test-helpers")]
use reqwest::Client;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Child,
};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

pub type BoxError = Box<dyn StdError + Send + Sync>;
pub type ShutdownSignal = Pin<Box<dyn Future<Output = ()> + Send>>;

pub fn init_tracing() {
    let _ = tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .without_time(),
        )
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .try_init();
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

#[cfg(feature = "test-helpers")]
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Stdio;

    #[cfg(feature = "test-helpers")]
    use reqwest::Client;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        process::Command,
        time::timeout,
    };

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

    #[cfg(feature = "test-helpers")]
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

    #[cfg(feature = "test-helpers")]
    #[tokio::test]
    async fn wait_for_health_reports_failures_after_exhausting_retries() {
        let client = Client::builder().build().expect("test client should build");

        let error = wait_for_health(&client, "http://127.0.0.1:9", 2, Duration::from_millis(1))
            .await
            .expect_err("unreachable health endpoints should fail");

        assert!(error.to_string().contains("health check did not succeed"));
    }

    #[cfg(feature = "test-helpers")]
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
