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
mod tests;
