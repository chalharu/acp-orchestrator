use std::ffi::OsString;

use acp_app_support::{
    BoxError, ListenerSetupError, RuntimeListenArgs, bind_listener, listener_endpoint,
    print_startup_line, shutdown_signal, wait_for_health,
};
use clap::Parser;
use reqwest::Client;
use snafu::prelude::*;

use crate::{AppState, MockClientError, ServerConfig, serve_with_shutdown};

type Result<T, E = BackendAppError> = std::result::Result<T, E>;
const READY_CHECK_ATTEMPTS: usize = 50;
const READY_CHECK_DELAY: std::time::Duration = std::time::Duration::from_millis(100);
const READY_CHECK_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(500);

#[derive(Debug, Snafu)]
pub enum BackendAppError {
    #[snafu(display("parsing backend CLI arguments failed: {source}"))]
    ParseArgs { source: clap::Error },

    #[snafu(transparent)]
    Setup { source: ListenerSetupError },

    #[snafu(display("building backend state failed"))]
    BuildState { source: MockClientError },

    #[snafu(display("building the backend readiness HTTP client failed"))]
    BuildHttpClient { source: reqwest::Error },

    #[snafu(display("waiting for the web backend readiness probe failed"))]
    WaitForReady { source: BoxError },

    #[snafu(display("running the web backend failed"))]
    Run { source: std::io::Error },
}

#[derive(Debug, Parser)]
#[command(name = "acp-web-backend")]
#[command(about = "ACP Orchestrator web backend")]
struct Cli {
    #[command(flatten)]
    listen: RuntimeListenArgs,
    #[arg(long, default_value_t = 8080)]
    port: u16,
    #[arg(long, default_value_t = 8)]
    session_cap: usize,
    #[arg(long, env = "ACP_MOCK_ADDRESS")]
    mock_address: String,
}

async fn run(cli: Cli) -> Result<()> {
    let listener = bind_listener(&cli.listen.host, cli.port, "web backend")
        .await
        .map_err(|source| BackendAppError::Setup { source })?;
    let endpoint = listener_endpoint(&listener, "web backend", "http://")
        .map_err(|source| BackendAppError::Setup { source })?;

    let state = AppState::new(ServerConfig {
        session_cap: cli.session_cap,
        mock_address: cli.mock_address,
    })
    .context(BuildStateSnafu)?;
    let client = Client::builder()
        .timeout(READY_CHECK_TIMEOUT)
        .build()
        .context(BuildHttpClientSnafu)?;
    let ready = wait_for_health(&client, &endpoint, READY_CHECK_ATTEMPTS, READY_CHECK_DELAY);
    let serve = serve_with_shutdown(listener, state, shutdown_signal(cli.listen.exit_after_ms));
    tokio::pin!(ready);
    tokio::pin!(serve);

    tokio::select! {
        result = &mut ready => {
            result.context(WaitForReadySnafu)?;
            print_startup_line("web backend", &endpoint);
            serve.await.context(RunSnafu)
        }
        result = &mut serve => result.context(RunSnafu),
    }
}

pub async fn run_with_args<I, T>(args: I) -> Result<()>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = Cli::try_parse_from(args).context(ParseArgsSnafu)?;
    run(cli).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn run_with_args_can_shutdown_cleanly() {
        run_with_args([
            "acp-web-backend",
            "--port",
            "0",
            "--mock-address",
            "127.0.0.1:9",
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
            "--mock-address",
            "127.0.0.1:9",
        ]));

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        handle.abort();
        let _ = handle.await;
    }

    #[tokio::test]
    async fn run_with_args_requires_a_mock_address() {
        let error = run_with_args(["acp-web-backend"])
            .await
            .expect_err("missing mock addresses should fail");

        assert!(matches!(error, BackendAppError::ParseArgs { .. }));
    }

    #[tokio::test]
    async fn run_with_args_reports_bind_failures() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let port = listener
            .local_addr()
            .expect("listener should expose its address")
            .port();

        let error = run_with_args([
            "acp-web-backend",
            "--port",
            &port.to_string(),
            "--mock-address",
            "127.0.0.1:9",
            "--exit-after-ms",
            "1",
        ])
        .await
        .expect_err("occupied ports should fail");

        assert!(matches!(
            error,
            BackendAppError::Setup {
                source: ListenerSetupError::Bind {
                    port: bound_port, ..
                }
            } if bound_port == port
        ));
    }
}
