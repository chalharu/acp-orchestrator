use acp_web_backend::{AppState, MockClientError, ServerConfig, serve};
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
    println!("web backend listening on http://{address}");

    let state = AppState::new(ServerConfig {
        session_cap: cli.session_cap,
        mock_url: cli.mock_url,
    })
    .context(BuildStateSnafu)?;

    serve(listener, state).await.context(RunSnafu)
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
