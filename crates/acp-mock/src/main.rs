use std::{
    ffi::OsString,
    future::{Future, pending},
    pin::Pin,
    time::Duration,
};

use acp_mock::{MockConfig, serve_with_shutdown};
use clap::Parser;
use snafu::prelude::*;
use tokio::net::TcpListener;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

type Result<T, E = MockError> = std::result::Result<T, E>;

#[derive(Debug, Snafu)]
enum MockError {
    #[snafu(display("binding the mock server on {host}:{port} failed"))]
    Bind {
        source: std::io::Error,
        host: String,
        port: u16,
    },

    #[snafu(display("reading the bound mock address failed"))]
    ReadBoundAddress { source: std::io::Error },

    #[snafu(display("running the mock server failed"))]
    Run { source: std::io::Error },
}

#[derive(Debug, Parser)]
#[command(name = "acp-mock")]
#[command(about = "ACP mock service")]
struct Cli {
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    #[arg(long, default_value_t = 8090)]
    port: u16,
    #[arg(long, default_value_t = 120)]
    response_delay_ms: u64,
    #[arg(long, hide = true)]
    exit_after_ms: Option<u64>,
}

async fn run(cli: Cli) -> Result<()> {
    init_tracing();

    let listener = TcpListener::bind((cli.host.as_str(), cli.port))
        .await
        .context(BindSnafu {
            host: cli.host.clone(),
            port: cli.port,
        })?;
    let address = listener.local_addr().context(ReadBoundAddressSnafu)?;
    println!("acp mock listening on http://{address}");

    let config = MockConfig {
        response_delay: Duration::from_millis(cli.response_delay_ms),
    };

    let shutdown: Pin<Box<dyn Future<Output = ()> + Send>> =
        if let Some(exit_after_ms) = cli.exit_after_ms {
            Box::pin(tokio::time::sleep(Duration::from_millis(exit_after_ms)))
        } else {
            Box::pin(pending())
        };

    serve_with_shutdown(listener, config, shutdown)
        .await
        .context(RunSnafu)
}

async fn run_with_args<I, T>(args: I) -> Result<()>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    run(Cli::parse_from(args)).await
}

#[tokio::main]
async fn main() -> Result<()> {
    run_with_args(std::env::args_os()).await
}

fn init_tracing() {
    let _ = tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .without_time(),
        )
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .try_init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn run_with_args_can_shutdown_cleanly() {
        run_with_args([
            "acp-mock",
            "--port",
            "0",
            "--response-delay-ms",
            "1",
            "--exit-after-ms",
            "50",
        ])
        .await
        .expect("mock server should stop cleanly");
    }

    #[tokio::test]
    async fn run_with_args_can_start_without_a_test_shutdown() {
        let handle = tokio::spawn(run_with_args([
            "acp-mock",
            "--port",
            "0",
            "--response-delay-ms",
            "1",
        ]));

        tokio::time::sleep(Duration::from_millis(50)).await;
        handle.abort();
        let _ = handle.await;
    }
}
