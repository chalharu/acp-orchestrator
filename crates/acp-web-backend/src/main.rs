use std::{
    ffi::OsString,
    future::{Future, pending},
    pin::Pin,
};

use acp_web_backend::{AppState, MockClientError, ServerConfig, serve_with_shutdown};
use clap::Parser;
use snafu::prelude::*;
use tokio::net::TcpListener;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

type Result<T, E = BackendError> = std::result::Result<T, E>;

#[derive(Debug, Snafu)]
enum BackendError {
    #[snafu(display("binding the web backend on {host}:{port} failed"))]
    Bind {
        source: std::io::Error,
        host: String,
        port: u16,
    },

    #[snafu(display("reading the bound backend address failed"))]
    ReadBoundAddress { source: std::io::Error },

    #[snafu(display("building backend state failed"))]
    BuildState { source: MockClientError },

    #[snafu(display("running the web backend failed"))]
    Run { source: std::io::Error },
}

#[derive(Debug, Parser)]
#[command(name = "acp-web-backend")]
#[command(about = "ACP Orchestrator web backend")]
struct Cli {
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    #[arg(long, default_value_t = 8080)]
    port: u16,
    #[arg(long, default_value_t = 8)]
    session_cap: usize,
    #[arg(long, env = "ACP_MOCK_URL")]
    mock_url: String,
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
    println!("web backend listening on http://{address}");

    let state = AppState::new(ServerConfig {
        session_cap: cli.session_cap,
        mock_url: cli.mock_url,
    })
    .context(BuildStateSnafu)?;

    let shutdown: Pin<Box<dyn Future<Output = ()> + Send>> =
        if let Some(exit_after_ms) = cli.exit_after_ms {
            Box::pin(tokio::time::sleep(std::time::Duration::from_millis(
                exit_after_ms,
            )))
        } else {
            Box::pin(pending())
        };

    serve_with_shutdown(listener, state, shutdown)
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
            "acp-web-backend",
            "--port",
            "0",
            "--mock-url",
            "http://127.0.0.1:9",
            "--exit-after-ms",
            "50",
        ])
        .await
        .expect("backend server should stop cleanly");
    }

    #[tokio::test]
    async fn run_with_args_can_start_without_a_test_shutdown() {
        let handle = tokio::spawn(run_with_args([
            "acp-web-backend",
            "--port",
            "0",
            "--mock-url",
            "http://127.0.0.1:9",
        ]));

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        handle.abort();
        let _ = handle.await;
    }
}
