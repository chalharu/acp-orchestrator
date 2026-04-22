use std::{
    future::{Future, pending},
    io,
    pin::Pin,
    time::Duration,
};

use clap::Args;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Child,
};

use acp_app_support_errors::BoxError;

pub type ShutdownSignal = Pin<Box<dyn Future<Output = ()> + Send>>;

#[derive(Debug, Args, Clone)]
pub struct RuntimeListenArgs {
    #[arg(long, default_value = "127.0.0.1")]
    pub host: String,
    #[arg(long, hide = true)]
    pub exit_after_ms: Option<u64>,
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

#[cfg(test)]
mod tests {
    use std::{process::Stdio, time::Duration};

    use tokio::{process::Command, time::timeout};

    use super::{read_startup_url, shutdown_signal};

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
}
