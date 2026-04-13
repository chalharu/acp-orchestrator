use std::{ffi::OsString, time::Duration};

use acp_app_support::{ListenerSetupError, RuntimeListenArgs, bind_listener, shutdown_signal};
use clap::Parser;
use snafu::prelude::*;

use crate::{MockConfig, serve_with_shutdown};

type Result<T, E = MockAppError> = std::result::Result<T, E>;

#[derive(Debug, Snafu)]
pub enum MockAppError {
    #[snafu(display("parsing mock CLI arguments failed: {source}"))]
    ParseArgs { source: clap::Error },

    #[snafu(transparent)]
    Setup { source: ListenerSetupError },

    #[snafu(display("running the mock server failed"))]
    Run { source: std::io::Error },
}

#[derive(Debug, Parser)]
#[command(name = "acp-mock")]
#[command(about = "ACP mock service")]
struct Cli {
    #[command(flatten)]
    listen: RuntimeListenArgs,
    #[arg(long, default_value_t = 8090)]
    port: u16,
    #[arg(long, default_value_t = 120)]
    response_delay_ms: u64,
}

async fn run(cli: Cli) -> Result<()> {
    let listener = bind_listener(&cli.listen.host, cli.port, "mock server", "acp mock")
        .await
        .map_err(|source| MockAppError::Setup { source })?;

    let config = MockConfig {
        response_delay: Duration::from_millis(cli.response_delay_ms),
    };

    serve_with_shutdown(listener, config, shutdown_signal(cli.listen.exit_after_ms))
        .await
        .context(RunSnafu)
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

    #[tokio::test]
    async fn run_with_args_rejects_invalid_delay_values() {
        let error = run_with_args(["acp-mock", "--response-delay-ms", "not-a-number"])
            .await
            .expect_err("invalid delay values should fail");

        assert!(matches!(error, MockAppError::ParseArgs { .. }));
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
            "acp-mock",
            "--port",
            &port.to_string(),
            "--exit-after-ms",
            "1",
        ])
        .await
        .expect_err("occupied ports should fail");

        assert!(matches!(
            error,
            MockAppError::Setup {
                source: ListenerSetupError::Bind {
                    port: bound_port, ..
                }
            } if bound_port == port
        ));
    }
}
