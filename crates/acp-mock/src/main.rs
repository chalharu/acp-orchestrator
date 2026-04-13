use std::time::Duration;

use acp_mock::{MockConfig, serve};
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
}

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let cli = Cli::parse();
    let listener = TcpListener::bind((cli.host.as_str(), cli.port))
        .await
        .context(BindSnafu {
            host: cli.host.clone(),
            port: cli.port,
        })?;
    let address = listener.local_addr().context(ReadBoundAddressSnafu)?;
    println!("acp mock listening on http://{address}");

    serve(
        listener,
        MockConfig {
            response_delay: Duration::from_millis(cli.response_delay_ms),
        },
    )
    .await
    .context(RunSnafu)
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
