use std::{ffi::OsString, time::Duration};

use clap::Parser;
use snafu::prelude::*;

use crate::support::errors::{BoxError, ListenerSetupError, ServiceReadinessError};
use crate::support::http::wait_for_tcp_connect;
use crate::support::runtime::{
    RuntimeListenArgs, bind_listener, listener_endpoint, print_startup_line,
};
use crate::support::service::{run_service_with_readiness, shutdown_signal};
use crate::{MockConfig, serve_with_shutdown};

type Result<T, E = MockAppError> = std::result::Result<T, E>;
const READY_CHECK_ATTEMPTS: usize = 300;
const READY_CHECK_DELAY: Duration = Duration::from_millis(100);

fn map_service_readiness_error(error: ServiceReadinessError<BoxError>) -> MockAppError {
    match error {
        ServiceReadinessError::Ready(source) => MockAppError::WaitForReady { source },
        ServiceReadinessError::Run(source) => MockAppError::Run { source },
    }
}

#[derive(Debug, Snafu)]
pub enum MockAppError {
    #[snafu(display("parsing mock CLI arguments failed: {source}"))]
    ParseArgs { source: clap::Error },

    #[snafu(transparent)]
    Setup { source: ListenerSetupError },

    #[snafu(display("running the mock server failed"))]
    Run { source: std::io::Error },

    #[snafu(display("waiting for the mock readiness probe failed"))]
    WaitForReady { source: BoxError },
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
    #[arg(long, default_value_t = false)]
    startup_hints: bool,
}

async fn run(cli: Cli) -> Result<()> {
    let listener = bind_listener(&cli.listen.host, cli.port, "mock server")
        .await
        .map_err(|source| MockAppError::Setup { source })?;
    let endpoint = listener_endpoint(&listener, "mock server", "")
        .map_err(|source| MockAppError::Setup { source })?;

    let config = MockConfig {
        response_delay: Duration::from_millis(cli.response_delay_ms),
        startup_hints: cli.startup_hints,
    };
    let ready = wait_for_tcp_connect(&endpoint, READY_CHECK_ATTEMPTS, READY_CHECK_DELAY);
    let serve = serve_with_shutdown(listener, config, shutdown_signal(cli.listen.exit_after_ms));

    run_service_with_readiness(ready, serve, || print_startup_line("acp mock", &endpoint))
        .await
        .map_err(map_service_readiness_error)
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

    #[test]
    fn service_readiness_errors_map_to_wait_for_ready_failures() {
        let error = map_service_readiness_error(ServiceReadinessError::Ready(
            std::io::Error::other("not ready").into(),
        ));

        assert!(matches!(error, MockAppError::WaitForReady { .. }));
    }

    #[test]
    fn service_readiness_errors_map_to_runtime_failures() {
        let error =
            map_service_readiness_error(ServiceReadinessError::Run(std::io::Error::other("boom")));

        assert!(matches!(error, MockAppError::Run { .. }));
    }

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
        let local_set = tokio::task::LocalSet::new();
        let result = tokio::time::timeout(
            Duration::from_millis(50),
            local_set.run_until(run_with_args([
                "acp-mock",
                "--port",
                "0",
                "--response-delay-ms",
                "1",
            ])),
        )
        .await;

        assert!(result.is_err(), "mock server should keep running");
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
