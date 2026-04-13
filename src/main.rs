use std::{
    env,
    ffi::OsString,
    path::PathBuf,
    process::{ExitStatus, Stdio},
    time::Duration,
};

use acp_mock::{MockConfig, serve as serve_mock};
use acp_web_backend::{AppState, MockClientError, ServerConfig, serve as serve_backend};
use reqwest::Client;
use snafu::prelude::*;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    net::TcpListener,
    process::{Child, Command},
    time::{Instant, sleep},
};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

type Result<T, E = LauncherError> = std::result::Result<T, E>;

#[derive(Debug, Snafu)]
enum LauncherError {
    #[snafu(display("reading the current executable path failed"))]
    CurrentExecutable { source: std::io::Error },

    #[snafu(display("building the launcher HTTP client failed"))]
    BuildHttpClient { source: reqwest::Error },

    #[snafu(display("spawning the {role} child process failed"))]
    SpawnChild {
        source: std::io::Error,
        role: &'static str,
    },

    #[snafu(display("checking the {role} child process status failed"))]
    CheckChildStatus {
        source: std::io::Error,
        role: &'static str,
    },

    #[snafu(display("{role} exited before becoming healthy"))]
    ChildExitedEarly {
        role: &'static str,
        status: ExitStatus,
    },

    #[snafu(display("{role} did not become healthy at {url}"))]
    WaitForHealth { role: &'static str, url: String },

    #[snafu(display("waiting for the {role} child process failed"))]
    WaitForChild {
        source: std::io::Error,
        role: &'static str,
    },

    #[snafu(display("capturing the {role} child stdout failed"))]
    MissingChildStdout { role: &'static str },

    #[snafu(display("reading the {role} startup line failed"))]
    ReadStartupLine {
        source: std::io::Error,
        role: &'static str,
    },

    #[snafu(display("timed out waiting for the {role} startup line"))]
    WaitForStartupLine { role: &'static str },

    #[snafu(display("the {role} startup line was invalid: {line}"))]
    InvalidStartupLine { role: &'static str, line: String },

    #[snafu(display("{role} exited with status code {code:?}"))]
    ChildExit {
        role: &'static str,
        code: Option<i32>,
    },

    #[snafu(display("terminating the {role} child process failed"))]
    TerminateChild {
        source: std::io::Error,
        role: &'static str,
    },

    #[snafu(display("missing the internal role name"))]
    MissingInternalRole,

    #[snafu(display("unknown internal role `{role}`"))]
    UnknownInternalRole { role: String },

    #[snafu(display("invalid internal arguments for {role}: {message}"))]
    InvalidInternalArguments { role: &'static str, message: String },

    #[snafu(display("binding the mock child on {host}:{port} failed"))]
    BindMock {
        source: std::io::Error,
        host: String,
        port: u16,
    },

    #[snafu(display("reading the bound mock child address failed"))]
    ReadMockBoundAddress { source: std::io::Error },

    #[snafu(display("binding the backend child on {host}:{port} failed"))]
    BindBackend {
        source: std::io::Error,
        host: String,
        port: u16,
    },

    #[snafu(display("reading the bound backend child address failed"))]
    ReadBackendBoundAddress { source: std::io::Error },

    #[snafu(display("building the backend child state failed"))]
    BuildBackendState { source: MockClientError },

    #[snafu(display("running the mock child failed"))]
    RunMock { source: std::io::Error },

    #[snafu(display("running the backend child failed"))]
    RunBackend { source: std::io::Error },

    #[snafu(display("running the cli child failed: {message}"))]
    RunCli { message: String },
}

struct SpawnedService {
    child: Child,
    base_url: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let args = env::args_os().collect::<Vec<_>>();
    if args.get(1).and_then(|arg| arg.to_str()) == Some("__internal-role") {
        let role = args
            .get(2)
            .cloned()
            .ok_or_else(|| MissingInternalRoleSnafu.build())?;
        let role_args = args.into_iter().skip(3).collect::<Vec<_>>();
        return run_internal_role(role, role_args).await;
    }

    let current_executable = env::current_exe().context(CurrentExecutableSnafu)?;
    let cli_args = forwarded_cli_args(&args);
    let client = Client::builder()
        .timeout(Duration::from_millis(200))
        .build()
        .context(BuildHttpClientSnafu)?;

    let SpawnedService {
        child: mut mock,
        base_url: mock_url,
    } = spawn_background_role(
        &current_executable,
        "acp mock",
        "mock",
        vec!["--port".into(), "0".into()],
        &[],
    )
    .await?;
    wait_for_health(&client, &mut mock, "acp mock", &mock_url).await?;

    let SpawnedService {
        child: mut backend,
        base_url: backend_url,
    } = spawn_background_role(
        &current_executable,
        "web backend",
        "backend",
        vec![
            "--port".into(),
            "0".into(),
            "--mock-url".into(),
            mock_url.clone().into(),
        ],
        &[],
    )
    .await?;
    wait_for_health(&client, &mut backend, "web backend", &backend_url).await?;

    let cli_status = spawn_foreground_role(
        &current_executable,
        "cli frontend",
        "cli",
        cli_args,
        &[("ACP_SERVER_URL", backend_url.as_str())],
    )
    .await?;

    terminate_child(&mut backend, "web backend").await?;
    terminate_child(&mut mock, "acp mock").await?;

    ensure_success("cli frontend", cli_status)
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

fn forwarded_cli_args(all_args: &[OsString]) -> Vec<OsString> {
    let args = all_args.iter().skip(1).cloned().collect::<Vec<_>>();
    if args.is_empty() {
        vec!["chat".into(), "--new".into()]
    } else {
        args
    }
}

async fn spawn_background_role(
    current_executable: &PathBuf,
    role_label: &'static str,
    role_name: &'static str,
    role_args: Vec<OsString>,
    envs: &[(&str, &str)],
) -> Result<SpawnedService> {
    let mut command = role_command(current_executable, role_name, role_args, envs);
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::inherit());
    command.kill_on_drop(true);
    let mut child = command
        .spawn()
        .context(SpawnChildSnafu { role: role_label })?;
    let base_url = read_startup_url(&mut child, role_label).await?;
    Ok(SpawnedService { child, base_url })
}

async fn spawn_foreground_role(
    current_executable: &PathBuf,
    role_label: &'static str,
    role_name: &'static str,
    role_args: Vec<OsString>,
    envs: &[(&str, &str)],
) -> Result<ExitStatus> {
    let mut child = role_command(current_executable, role_name, role_args, envs)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .context(SpawnChildSnafu { role: role_label })?;

    child
        .wait()
        .await
        .context(WaitForChildSnafu { role: role_label })
}

fn role_command(
    current_executable: &PathBuf,
    role_name: &'static str,
    role_args: Vec<OsString>,
    envs: &[(&str, &str)],
) -> Command {
    let mut command = Command::new(current_executable);
    command.arg("__internal-role").arg(role_name);

    for arg in role_args {
        command.arg(arg);
    }

    for (key, value) in envs {
        command.env(key, value);
    }

    command
}

async fn read_startup_url(child: &mut Child, role: &'static str) -> Result<String> {
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| MissingChildStdoutSnafu { role }.build())?;
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    let bytes_read = tokio::time::timeout(Duration::from_secs(15), reader.read_line(&mut line))
        .await
        .map_err(|_| LauncherError::WaitForStartupLine { role })?
        .context(ReadStartupLineSnafu { role })?;

    if bytes_read == 0 {
        return InvalidStartupLineSnafu {
            role,
            line: "<empty>".to_string(),
        }
        .fail();
    }

    let prefix = "listening on ";
    let line = line.trim();
    let base_url = line
        .split_once(prefix)
        .map(|(_, value)| value.to_string())
        .ok_or_else(|| {
            InvalidStartupLineSnafu {
                role,
                line: line.to_string(),
            }
            .build()
        })?;

    Ok(base_url)
}

async fn wait_for_health(
    client: &Client,
    child: &mut Child,
    role: &'static str,
    base_url: &str,
) -> Result<()> {
    let health_url = format!("{base_url}/healthz");
    let deadline = Instant::now() + Duration::from_secs(15);

    loop {
        if let Some(status) = child.try_wait().context(CheckChildStatusSnafu { role })? {
            return ChildExitedEarlySnafu { role, status }.fail();
        }

        if let Ok(response) = client.get(&health_url).send().await
            && response.status().is_success()
        {
            return Ok(());
        }

        if Instant::now() >= deadline {
            return WaitForHealthSnafu {
                role,
                url: health_url,
            }
            .fail();
        }

        sleep(Duration::from_millis(100)).await;
    }
}

async fn terminate_child(child: &mut Child, role: &'static str) -> Result<()> {
    if child
        .try_wait()
        .context(CheckChildStatusSnafu { role })?
        .is_some()
    {
        return Ok(());
    }

    child.kill().await.context(TerminateChildSnafu { role })?;
    let _ = child.wait().await.context(WaitForChildSnafu { role })?;
    Ok(())
}

fn ensure_success(role: &'static str, status: ExitStatus) -> Result<()> {
    if status.success() {
        Ok(())
    } else {
        ChildExitSnafu {
            role,
            code: status.code(),
        }
        .fail()
    }
}

async fn run_internal_role(role: OsString, role_args: Vec<OsString>) -> Result<()> {
    match role.to_string_lossy().as_ref() {
        "cli" => {
            let args = std::iter::once(OsString::from("acp")).chain(role_args);
            acp_cli::run_with_args(args)
                .await
                .map_err(|error| LauncherError::RunCli {
                    message: error.to_string(),
                })
        }
        "mock" => run_mock_role(role_args).await,
        "backend" => run_backend_role(role_args).await,
        value => UnknownInternalRoleSnafu {
            role: value.to_string(),
        }
        .fail(),
    }
}

async fn run_mock_role(role_args: Vec<OsString>) -> Result<()> {
    let mut host = "127.0.0.1".to_string();
    let mut port = 8090u16;
    let mut response_delay_ms = 120u64;
    let args = to_strings("acp mock", role_args)?;
    let mut iter = args.into_iter();

    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--host" => host = next_value("acp mock", &mut iter, "--host")?,
            "--port" => port = parse_u16("acp mock", &mut iter, "--port")?,
            "--response-delay-ms" => {
                response_delay_ms = parse_u64("acp mock", &mut iter, "--response-delay-ms")?
            }
            other => {
                return InvalidInternalArgumentsSnafu {
                    role: "acp mock",
                    message: format!("unexpected argument `{other}`"),
                }
                .fail();
            }
        }
    }

    let listener = TcpListener::bind((host.as_str(), port))
        .await
        .context(BindMockSnafu {
            host: host.clone(),
            port,
        })?;
    let address = listener.local_addr().context(ReadMockBoundAddressSnafu)?;
    println!("acp mock listening on http://{address}");

    serve_mock(
        listener,
        MockConfig {
            response_delay: Duration::from_millis(response_delay_ms),
        },
    )
    .await
    .context(RunMockSnafu)
}

async fn run_backend_role(role_args: Vec<OsString>) -> Result<()> {
    let mut host = "127.0.0.1".to_string();
    let mut port = 8080u16;
    let mut session_cap = 8usize;
    let mut mock_url = None;
    let args = to_strings("web backend", role_args)?;
    let mut iter = args.into_iter();

    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--host" => host = next_value("web backend", &mut iter, "--host")?,
            "--port" => port = parse_u16("web backend", &mut iter, "--port")?,
            "--session-cap" => {
                session_cap = parse_usize("web backend", &mut iter, "--session-cap")?
            }
            "--mock-url" => mock_url = Some(next_value("web backend", &mut iter, "--mock-url")?),
            other => {
                return InvalidInternalArgumentsSnafu {
                    role: "web backend",
                    message: format!("unexpected argument `{other}`"),
                }
                .fail();
            }
        }
    }

    let mock_url = mock_url.ok_or_else(|| {
        InvalidInternalArgumentsSnafu {
            role: "web backend",
            message: "missing `--mock-url`".to_string(),
        }
        .build()
    })?;

    let listener = TcpListener::bind((host.as_str(), port))
        .await
        .context(BindBackendSnafu {
            host: host.clone(),
            port,
        })?;
    let address = listener
        .local_addr()
        .context(ReadBackendBoundAddressSnafu)?;
    println!("web backend listening on http://{address}");

    let state = AppState::new(ServerConfig {
        session_cap,
        mock_url,
    })
    .context(BuildBackendStateSnafu)?;

    serve_backend(listener, state)
        .await
        .context(RunBackendSnafu)
}

fn to_strings(role: &'static str, args: Vec<OsString>) -> Result<Vec<String>> {
    args.into_iter()
        .map(|arg| {
            arg.into_string()
                .map_err(|value| LauncherError::InvalidInternalArguments {
                    role,
                    message: format!("non-UTF-8 argument: {:?}", value),
                })
        })
        .collect()
}

fn next_value(
    role: &'static str,
    iter: &mut impl Iterator<Item = String>,
    flag: &'static str,
) -> Result<String> {
    iter.next().ok_or_else(|| {
        InvalidInternalArgumentsSnafu {
            role,
            message: format!("missing value for `{flag}`"),
        }
        .build()
    })
}

fn parse_u16(
    role: &'static str,
    iter: &mut impl Iterator<Item = String>,
    flag: &'static str,
) -> Result<u16> {
    let value = next_value(role, iter, flag)?;
    value
        .parse::<u16>()
        .map_err(|_| LauncherError::InvalidInternalArguments {
            role,
            message: format!("`{flag}` expects a u16 value"),
        })
}

fn parse_u64(
    role: &'static str,
    iter: &mut impl Iterator<Item = String>,
    flag: &'static str,
) -> Result<u64> {
    let value = next_value(role, iter, flag)?;
    value
        .parse::<u64>()
        .map_err(|_| LauncherError::InvalidInternalArguments {
            role,
            message: format!("`{flag}` expects a u64 value"),
        })
}

fn parse_usize(
    role: &'static str,
    iter: &mut impl Iterator<Item = String>,
    flag: &'static str,
) -> Result<usize> {
    let value = next_value(role, iter, flag)?;
    value
        .parse::<usize>()
        .map_err(|_| LauncherError::InvalidInternalArguments {
            role,
            message: format!("`{flag}` expects a usize value"),
        })
}
