use std::{env, ffi::OsString, path::PathBuf, sync::Arc};

use clap::Parser;
use snafu::prelude::*;

use crate::support::errors::{BoxError, ListenerSetupError, ServiceReadinessError};
use crate::support::http::{build_http_client_for_url, wait_for_health, wait_for_http_success};
use crate::support::runtime::{
    RuntimeListenArgs, bind_listener, listener_endpoint, print_startup_line,
};
use crate::support::service::{run_service_with_readiness, shutdown_signal};
use crate::{
    AppState, AppStateBuildError, ServerConfig, serve_with_shutdown,
    workspace_repository::WorkspaceRepository, workspace_store::SqliteWorkspaceRepository,
};

type Result<T, E = BackendAppError> = std::result::Result<T, E>;
const READY_CHECK_ATTEMPTS: usize = 50;
const READY_CHECK_DELAY: std::time::Duration = std::time::Duration::from_millis(100);
const READY_CHECK_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(500);

async fn wait_for_app_entrypoint(client: &reqwest::Client, base_url: &str) -> Result<(), BoxError> {
    let app_url = format!("{base_url}/app/");
    wait_for_http_success(
        client,
        &app_url,
        READY_CHECK_ATTEMPTS,
        READY_CHECK_DELAY,
        "browser entrypoint",
    )
    .await
}

fn map_service_readiness_error(error: ServiceReadinessError<BoxError>) -> BackendAppError {
    match error {
        ServiceReadinessError::Ready(source) => BackendAppError::WaitForReady { source },
        ServiceReadinessError::Run(source) => BackendAppError::Run { source },
    }
}

#[derive(Debug, Snafu)]
pub enum BackendAppError {
    #[snafu(display("parsing backend CLI arguments failed: {source}"))]
    ParseArgs { source: clap::Error },

    #[snafu(transparent)]
    Setup { source: ListenerSetupError },

    #[snafu(display("building backend state failed"))]
    BuildState { source: AppStateBuildError },

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
    #[arg(long, alias = "mock-address", env = "ACP_SERVER")]
    acp_server: Option<String>,
    #[arg(long, default_value_t = false)]
    startup_hints: bool,
    #[arg(long, default_value = ".acp-state")]
    state_dir: PathBuf,
    /// Directory containing the Trunk-compiled Leptos CSR bundle.
    /// The backend serves the fingerprinted output through stable alias routes.
    /// When absent the WASM asset routes return 503 until the frontend is built.
    #[arg(long)]
    frontend_dist: Option<PathBuf>,
}

fn resolve_acp_server(
    acp_server: Option<String>,
    deprecated_mock_address: Option<String>,
) -> std::result::Result<String, clap::Error> {
    acp_server
        .or(deprecated_mock_address)
        .ok_or_else(|| {
            clap::Error::raw(
                clap::error::ErrorKind::MissingRequiredArgument,
                "missing ACP server address; use --acp-server, ACP_SERVER, or the deprecated ACP_MOCK_ADDRESS",
            )
        })
}

async fn run(cli: Cli) -> Result<()> {
    let acp_server = resolve_acp_server(cli.acp_server.clone(), env::var("ACP_MOCK_ADDRESS").ok())
        .context(ParseArgsSnafu)?;
    let listener = bind_listener(&cli.listen.host, cli.port, "web backend")
        .await
        .map_err(|source| BackendAppError::Setup { source })?;
    let endpoint = listener_endpoint(&listener, "web backend", "https://")
        .map_err(|source| BackendAppError::Setup { source })?;

    let config = ServerConfig {
        session_cap: cli.session_cap,
        acp_server,
        startup_hints: cli.startup_hints,
        state_dir: cli.state_dir,
        frontend_dist: cli.frontend_dist,
    };
    let workspace_repository: Arc<dyn WorkspaceRepository> = Arc::new(
        SqliteWorkspaceRepository::new(config.state_dir.join("db.sqlite"))
            .map_err(AppStateBuildError::from)
            .context(BuildStateSnafu)?,
    );
    let state = AppState::new(config, workspace_repository).context(BuildStateSnafu)?;
    let client = build_http_client_for_url(&endpoint, Some(READY_CHECK_TIMEOUT))
        .context(BuildHttpClientSnafu)?;
    let ready = async {
        wait_for_health(&client, &endpoint, READY_CHECK_ATTEMPTS, READY_CHECK_DELAY).await?;
        wait_for_app_entrypoint(&client, &endpoint).await
    };
    let serve = serve_with_shutdown(listener, state, shutdown_signal(cli.listen.exit_after_ms));

    run_service_with_readiness(ready, serve, || {
        print_startup_line("web backend", &endpoint)
    })
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

    fn test_cli(acp_server: Option<&str>) -> Cli {
        Cli {
            listen: RuntimeListenArgs {
                host: "127.0.0.1".to_string(),
                exit_after_ms: None,
            },
            port: 0,
            session_cap: 8,
            acp_server: acp_server.map(str::to_string),
            startup_hints: false,
            state_dir: PathBuf::from(".acp-state"),
            frontend_dist: None,
        }
    }

    #[test]
    fn service_readiness_errors_map_to_wait_for_ready_failures() {
        let error = map_service_readiness_error(ServiceReadinessError::Ready(
            std::io::Error::other("not ready").into(),
        ));

        assert!(matches!(error, BackendAppError::WaitForReady { .. }));
    }

    #[test]
    fn service_readiness_errors_map_to_runtime_failures() {
        let error =
            map_service_readiness_error(ServiceReadinessError::Run(std::io::Error::other("boom")));

        assert!(matches!(error, BackendAppError::Run { .. }));
    }

    #[test]
    fn resolve_acp_server_prefers_the_new_surface() {
        let cli = test_cli(Some("127.0.0.1:8090"));

        assert_eq!(
            resolve_acp_server(cli.acp_server.clone(), Some("127.0.0.1:9000".to_string()))
                .expect("the ACP server should resolve"),
            "127.0.0.1:8090"
        );
    }

    #[test]
    fn resolve_acp_server_accepts_the_deprecated_env_fallback() {
        let cli = test_cli(None);

        assert_eq!(
            resolve_acp_server(cli.acp_server.clone(), Some("127.0.0.1:8090".to_string()))
                .expect("the legacy ACP server should resolve"),
            "127.0.0.1:8090"
        );
    }

    #[tokio::test]
    async fn run_with_args_can_shutdown_cleanly() {
        run_with_args([
            "acp-web-backend",
            "--port",
            "0",
            "--acp-server",
            "127.0.0.1:9",
            "--exit-after-ms",
            "50",
        ])
        .await
        .expect("backend server should stop cleanly");
    }

    #[tokio::test]
    async fn run_with_args_accepts_the_deprecated_mock_address_flag() {
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
        .expect("the deprecated mock-address flag should still work");
    }

    #[tokio::test]
    async fn run_with_args_can_start_without_a_test_shutdown() {
        let handle = tokio::spawn(run_with_args([
            "acp-web-backend",
            "--port",
            "0",
            "--acp-server",
            "127.0.0.1:9",
        ]));

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        handle.abort();
        let _ = handle.await;
    }

    #[tokio::test]
    async fn run_with_args_requires_an_acp_server() {
        let error = run_with_args(["acp-web-backend"])
            .await
            .expect_err("missing ACP server addresses should fail");

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
            "--acp-server",
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
