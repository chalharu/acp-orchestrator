use std::{
    error::Error as StdError,
    fs,
    io::{self, Write},
    path::PathBuf,
};

use acp_orchestrator::{
    AppState, ServerConfig,
    models::{
        CloseSessionResponse, CreateSessionResponse, ErrorResponse, MessageRole, PromptRequest,
        PromptResponse, StreamEvent, StreamEventPayload,
    },
    serve, serve_with_shutdown,
};
use chrono::{DateTime, Utc};
use clap::{Args, Parser, Subcommand};
use eventsource_stream::Eventsource;
use futures_util::{StreamExt, pin_mut};
use reqwest::{Client, Response, StatusCode};
use serde::{Deserialize, Serialize};
use snafu::prelude::*;
use tokio::{net::TcpListener, sync::oneshot};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

type Result<T, E = CliError> = std::result::Result<T, E>;

#[derive(Debug, Snafu)]
enum CliError {
    #[snafu(display("choose either `--new` or `--session <id>`"))]
    ChatModeRequired,

    #[snafu(display(
        "{command} requires `--server-url` or ACP_SERVER_URL to point at a running backend"
    ))]
    MissingServerUrl { command: &'static str },

    #[snafu(display("building HTTP client failed"))]
    BuildHttpClient { source: reqwest::Error },

    #[snafu(display("binding backend on {host}:{port} failed"))]
    BindBackend {
        source: std::io::Error,
        host: String,
        port: u16,
    },

    #[snafu(display("reading bound backend address failed"))]
    ReadBoundAddress { source: std::io::Error },

    #[snafu(display("running backend failed"))]
    RunBackend { source: std::io::Error },

    #[snafu(display("binding embedded backend failed"))]
    BindEmbeddedBackend { source: std::io::Error },

    #[snafu(display("reading embedded backend address failed"))]
    ReadEmbeddedBackendAddress { source: std::io::Error },

    #[snafu(display("joining prompt reader task failed"))]
    JoinPromptReader { source: tokio::task::JoinError },

    #[snafu(display("flushing prompt failed"))]
    FlushPrompt { source: std::io::Error },

    #[snafu(display("reading prompt line failed"))]
    ReadPromptLine { source: std::io::Error },

    #[snafu(display("{action} request failed"))]
    SendRequest {
        source: reqwest::Error,
        action: &'static str,
    },

    #[snafu(display("{action} failed with HTTP {status}: {message}"))]
    HttpStatus {
        action: &'static str,
        status: StatusCode,
        message: String,
    },

    #[snafu(display("decoding {action} response failed"))]
    DecodeResponse {
        source: reqwest::Error,
        action: &'static str,
    },

    #[snafu(display("reading event stream failed"))]
    ReadEventStream {
        source: Box<dyn StdError + Send + Sync + 'static>,
    },

    #[snafu(display("decoding stream event failed"))]
    DecodeStreamEvent { source: serde_json::Error },

    #[snafu(display("unable to determine a recent-session cache directory"))]
    MissingRecentSessionDirectory,

    #[snafu(display("reading recent-session cache from {} failed", path.display()))]
    ReadRecentSessions {
        source: std::io::Error,
        path: PathBuf,
    },

    #[snafu(display("parsing recent-session cache from {} failed", path.display()))]
    ParseRecentSessions {
        source: serde_json::Error,
        path: PathBuf,
    },

    #[snafu(display("creating recent-session cache directory {} failed", path.display()))]
    CreateRecentSessionsDirectory {
        source: std::io::Error,
        path: PathBuf,
    },

    #[snafu(display("serializing recent-session cache failed"))]
    SerializeRecentSessions { source: serde_json::Error },

    #[snafu(display("writing recent-session cache to {} failed", path.display()))]
    WriteRecentSessions {
        source: std::io::Error,
        path: PathBuf,
    },
}

#[derive(Parser, Debug)]
#[command(name = "acp")]
#[command(about = "ACP Orchestrator slice 1 CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    Serve(ServeArgs),
    Chat(ChatArgs),
    Session(SessionArgs),
}

#[derive(Args, Debug)]
struct ServeArgs {
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    #[arg(long, default_value_t = 8080)]
    port: u16,
    #[arg(long, default_value_t = 8)]
    session_cap: usize,
    #[arg(long, default_value_t = 120)]
    assistant_delay_ms: u64,
}

#[derive(Args, Debug)]
struct ChatArgs {
    #[arg(long, default_value_t = false)]
    new: bool,
    #[arg(long = "session")]
    session_id: Option<String>,
    #[arg(long, env = "ACP_SERVER_URL")]
    server_url: Option<String>,
    #[arg(long, env = "ACP_AUTH_TOKEN", default_value = "developer")]
    auth_token: String,
}

#[derive(Args, Debug)]
struct SessionArgs {
    #[command(subcommand)]
    command: SessionCommand,
}

#[derive(Subcommand, Debug)]
enum SessionCommand {
    List,
    Close(CloseArgs),
}

#[derive(Args, Debug)]
struct CloseArgs {
    session_id: String,
    #[arg(long, env = "ACP_SERVER_URL")]
    server_url: Option<String>,
    #[arg(long, env = "ACP_AUTH_TOKEN", default_value = "developer")]
    auth_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RecentSessionEntry {
    session_id: String,
    server_url: String,
    last_used_at: DateTime<Utc>,
}

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let cli = Cli::parse();
    match cli.command {
        Command::Serve(args) => run_serve(args).await,
        Command::Chat(args) => run_chat(args).await,
        Command::Session(args) => run_session(args).await,
    }
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

async fn run_serve(args: ServeArgs) -> Result<()> {
    let listener = TcpListener::bind((args.host.as_str(), args.port))
        .await
        .context(BindBackendSnafu {
            host: args.host.clone(),
            port: args.port,
        })?;
    let address = listener.local_addr().context(ReadBoundAddressSnafu)?;
    println!("slice1 backend listening on http://{address}");

    serve(
        listener,
        AppState::new(ServerConfig {
            session_cap: args.session_cap,
            assistant_delay: std::time::Duration::from_millis(args.assistant_delay_ms),
        }),
    )
    .await
    .context(RunBackendSnafu)
}

async fn run_chat(args: ChatArgs) -> Result<()> {
    ensure!(args.new || args.session_id.is_some(), ChatModeRequiredSnafu);

    let client = Client::builder().build().context(BuildHttpClientSnafu)?;

    let backend = if args.new {
        resolve_backend(args.server_url.clone()).await?
    } else {
        let server_url = args.server_url.clone().ok_or_else(|| {
            MissingServerUrlSnafu {
                command: "reattach",
            }
            .build()
        })?;
        ResolvedBackend {
            base_url: server_url,
            embedded: None,
        }
    };

    let session = if args.new {
        create_session(&client, &backend.base_url, &args.auth_token).await?
    } else {
        get_session(
            &client,
            &backend.base_url,
            &args.auth_token,
            args.session_id
                .as_deref()
                .expect("session id checked before chat execution"),
        )
        .await?
    };

    record_recent_session(&RecentSessionEntry {
        session_id: session.id.clone(),
        server_url: backend.base_url.clone(),
        last_used_at: Utc::now(),
    })?;

    println!("session: {}", session.id);
    println!("connected to backend: {}", backend.base_url);
    if backend.embedded.is_some() {
        println!("[status] started an embedded slice1 backend for this chat session");
    }

    let events_url = format!("{}/api/v1/sessions/{}/events", backend.base_url, session.id);
    let event_client = client.clone();
    let auth_token = args.auth_token.clone();
    let event_task = tokio::spawn(async move {
        if let Err(error) = stream_events(event_client, events_url, auth_token).await {
            eprintln!("[status] event stream ended: {error}");
        }
    });

    loop {
        let Some(line) = read_prompt_line().await? else {
            break;
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if trimmed.starts_with('/') {
            if handle_repl_command(trimmed).await? {
                break;
            }
            continue;
        }

        submit_prompt(
            &client,
            &backend.base_url,
            &args.auth_token,
            &session.id,
            trimmed,
        )
        .await?;
    }

    event_task.abort();
    Ok(())
}

async fn run_session(args: SessionArgs) -> Result<()> {
    match args.command {
        SessionCommand::List => {
            let entries = load_recent_sessions()?;
            if entries.is_empty() {
                println!("no recent sessions recorded");
                return Ok(());
            }

            for entry in entries {
                println!(
                    "{}\t{}\t{}",
                    entry.session_id,
                    entry.server_url,
                    entry.last_used_at.to_rfc3339()
                );
            }

            Ok(())
        }
        SessionCommand::Close(args) => {
            let server_url = args.server_url.ok_or_else(|| {
                MissingServerUrlSnafu {
                    command: "closing a session",
                }
                .build()
            })?;
            let client = Client::builder().build().context(BuildHttpClientSnafu)?;
            close_session(&client, &server_url, &args.auth_token, &args.session_id).await?;
            remove_recent_session(&args.session_id)?;
            println!("[status] session {} closed", args.session_id);
            Ok(())
        }
    }
}

async fn handle_repl_command(command: &str) -> Result<bool> {
    match command {
        "/help" => {
            println!("/help");
            println!("/quit");
            println!("/cancel (planned for slice 2)");
            println!("/approve <request-id> (planned for slice 2)");
            println!("/deny <request-id> (planned for slice 2)");
            Ok(false)
        }
        "/quit" => Ok(true),
        value if value.starts_with("/cancel") => {
            println!("[status] `/cancel` is planned for slice 2.");
            Ok(false)
        }
        value if value.starts_with("/approve ") => {
            println!("[status] `/approve` is planned for slice 2.");
            Ok(false)
        }
        value if value.starts_with("/deny ") => {
            println!("[status] `/deny` is planned for slice 2.");
            Ok(false)
        }
        _ => {
            println!("[status] unknown command. Use `/help`.");
            Ok(false)
        }
    }
}

async fn read_prompt_line() -> Result<Option<String>> {
    tokio::task::spawn_blocking(|| -> Result<Option<String>> {
        print!("> ");
        io::stdout().flush().context(FlushPromptSnafu)?;

        let mut buffer = String::new();
        let bytes_read = io::stdin()
            .read_line(&mut buffer)
            .context(ReadPromptLineSnafu)?;

        if bytes_read == 0 {
            Ok(None)
        } else {
            Ok(Some(buffer))
        }
    })
    .await
    .context(JoinPromptReaderSnafu)?
}

async fn resolve_backend(server_url: Option<String>) -> Result<ResolvedBackend> {
    if let Some(server_url) = server_url {
        return Ok(ResolvedBackend {
            base_url: server_url,
            embedded: None,
        });
    }

    let embedded = EmbeddedBackend::spawn().await?;
    let base_url = embedded.base_url.clone();
    Ok(ResolvedBackend {
        base_url,
        embedded: Some(embedded),
    })
}

async fn create_session(
    client: &Client,
    base_url: &str,
    auth_token: &str,
) -> Result<acp_orchestrator::SessionSnapshot> {
    let response = client
        .post(format!("{base_url}/api/v1/sessions"))
        .bearer_auth(auth_token)
        .send()
        .await
        .context(SendRequestSnafu {
            action: "create session",
        })?;
    let response = ensure_success(response, "create session").await?;
    let payload: CreateSessionResponse = response.json().await.context(DecodeResponseSnafu {
        action: "create session",
    })?;
    Ok(payload.session)
}

async fn get_session(
    client: &Client,
    base_url: &str,
    auth_token: &str,
    session_id: &str,
) -> Result<acp_orchestrator::SessionSnapshot> {
    let response = client
        .get(format!("{base_url}/api/v1/sessions/{session_id}"))
        .bearer_auth(auth_token)
        .send()
        .await
        .context(SendRequestSnafu {
            action: "load session",
        })?;
    let response = ensure_success(response, "load session").await?;
    let payload: CreateSessionResponse = response.json().await.context(DecodeResponseSnafu {
        action: "load session",
    })?;
    Ok(payload.session)
}

async fn submit_prompt(
    client: &Client,
    base_url: &str,
    auth_token: &str,
    session_id: &str,
    prompt: &str,
) -> Result<()> {
    let response = client
        .post(format!("{base_url}/api/v1/sessions/{session_id}/messages"))
        .bearer_auth(auth_token)
        .json(&PromptRequest {
            text: prompt.to_string(),
        })
        .send()
        .await
        .context(SendRequestSnafu {
            action: "submit prompt",
        })?;
    let response = ensure_success(response, "submit prompt").await?;
    let _: PromptResponse = response.json().await.context(DecodeResponseSnafu {
        action: "submit prompt",
    })?;
    Ok(())
}

async fn close_session(
    client: &Client,
    base_url: &str,
    auth_token: &str,
    session_id: &str,
) -> Result<CloseSessionResponse> {
    let response = client
        .post(format!("{base_url}/api/v1/sessions/{session_id}/close"))
        .bearer_auth(auth_token)
        .send()
        .await
        .context(SendRequestSnafu {
            action: "close session",
        })?;
    let response = ensure_success(response, "close session").await?;
    response.json().await.context(DecodeResponseSnafu {
        action: "close session",
    })
}

async fn ensure_success(response: Response, action: &'static str) -> Result<Response> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }

    let message = match response.json::<ErrorResponse>().await {
        Ok(payload) => payload.error,
        Err(_) => status
            .canonical_reason()
            .unwrap_or("request failed")
            .to_string(),
    };

    HttpStatusSnafu {
        action,
        status,
        message,
    }
    .fail()
}

async fn stream_events(client: Client, events_url: String, auth_token: String) -> Result<()> {
    let response = client
        .get(events_url)
        .bearer_auth(auth_token)
        .send()
        .await
        .context(SendRequestSnafu {
            action: "open event stream",
        })?;
    let response = ensure_success(response, "open event stream").await?;
    let stream = response.bytes_stream().eventsource();
    pin_mut!(stream);

    while let Some(event) = stream.next().await {
        let event = event.map_err(|source| CliError::ReadEventStream {
            source: Box::new(source),
        })?;
        let payload: StreamEvent =
            serde_json::from_str(&event.data).context(DecodeStreamEventSnafu)?;
        render_event(&payload);
    }

    Ok(())
}

fn render_event(event: &StreamEvent) {
    match &event.payload {
        StreamEventPayload::SessionSnapshot { session } => {
            if session.messages.is_empty() {
                println!("[status] session ready");
            } else {
                for message in &session.messages {
                    render_message(message.role.clone(), &message.text);
                }
            }
        }
        StreamEventPayload::ConversationMessage { message } => {
            render_message(message.role.clone(), &message.text);
        }
        StreamEventPayload::SessionClosed { reason, .. } => {
            println!("[status] session closed: {reason}");
        }
        StreamEventPayload::Status { message } => {
            println!("[status] {message}");
        }
    }
}

fn render_message(role: MessageRole, text: &str) {
    match role {
        MessageRole::User => println!("[user] {text}"),
        MessageRole::Assistant => println!("[assistant] {text}"),
    }
}

fn recent_sessions_path() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("ACP_RECENT_SESSIONS_PATH") {
        return Ok(PathBuf::from(path));
    }

    let mut directory =
        dirs::data_local_dir().ok_or_else(|| MissingRecentSessionDirectorySnafu.build())?;
    directory.push("acp-orchestrator");
    directory.push("recent-sessions.json");
    Ok(directory)
}

fn load_recent_sessions() -> Result<Vec<RecentSessionEntry>> {
    let path = recent_sessions_path()?;
    if !path.exists() {
        return Ok(Vec::new());
    }

    let raw = fs::read_to_string(&path).context(ReadRecentSessionsSnafu { path: path.clone() })?;
    let entries = serde_json::from_str(&raw).context(ParseRecentSessionsSnafu { path })?;
    Ok(entries)
}

fn save_recent_sessions(entries: &[RecentSessionEntry]) -> Result<()> {
    let path = recent_sessions_path()?;
    if let Some(parent) = path.parent() {
        let parent = parent.to_path_buf();
        fs::create_dir_all(&parent).context(CreateRecentSessionsDirectorySnafu { path: parent })?;
    }

    let serialized = serde_json::to_string_pretty(entries).context(SerializeRecentSessionsSnafu)?;
    fs::write(&path, serialized).context(WriteRecentSessionsSnafu { path })?;
    Ok(())
}

fn record_recent_session(entry: &RecentSessionEntry) -> Result<()> {
    let mut entries = load_recent_sessions()?;
    entries.retain(|existing| existing.session_id != entry.session_id);
    entries.insert(0, entry.clone());
    entries.truncate(20);
    save_recent_sessions(&entries)
}

fn remove_recent_session(session_id: &str) -> Result<()> {
    let mut entries = load_recent_sessions()?;
    entries.retain(|entry| entry.session_id != session_id);
    save_recent_sessions(&entries)
}

struct ResolvedBackend {
    base_url: String,
    embedded: Option<EmbeddedBackend>,
}

struct EmbeddedBackend {
    base_url: String,
    shutdown: Option<oneshot::Sender<()>>,
}

impl EmbeddedBackend {
    async fn spawn() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .context(BindEmbeddedBackendSnafu)?;
        let address = listener
            .local_addr()
            .context(ReadEmbeddedBackendAddressSnafu)?;
        let (shutdown_tx, shutdown_rx) = oneshot::channel();

        tokio::spawn(async move {
            let shutdown = async move {
                let _ = shutdown_rx.await;
            };

            if let Err(error) =
                serve_with_shutdown(listener, AppState::new(ServerConfig::default()), shutdown)
                    .await
            {
                eprintln!("[status] embedded backend stopped: {error}");
            }
        });

        Ok(Self {
            base_url: format!("http://{address}"),
            shutdown: Some(shutdown_tx),
        })
    }
}

impl Drop for EmbeddedBackend {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
    }
}
