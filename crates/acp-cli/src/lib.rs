use std::{
    error::Error as StdError,
    ffi::OsString,
    fs,
    io::{self, Write},
    path::PathBuf,
};

use acp_contracts::{
    CloseSessionResponse, CreateSessionResponse, ErrorResponse, MessageRole, PromptRequest,
    PromptResponse, SessionSnapshot, StreamEvent, StreamEventPayload,
};
use chrono::{DateTime, Utc};
use clap::{Args, Parser, Subcommand};
use eventsource_stream::Eventsource;
use futures_util::{StreamExt, pin_mut};
use reqwest::{Client, Response, StatusCode};
use serde::{Deserialize, Serialize};
use snafu::prelude::*;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

pub type Result<T, E = CliError> = std::result::Result<T, E>;

#[derive(Debug, Snafu)]
pub enum CliError {
    #[snafu(display("choose either `--new` or `--session <id>`"))]
    ChatModeRequired,

    #[snafu(display(
        "{command} requires `--server-url` or ACP_SERVER_URL to point at a running backend"
    ))]
    MissingServerUrl { command: &'static str },

    #[snafu(display("building the HTTP client failed"))]
    BuildHttpClient { source: reqwest::Error },

    #[snafu(display("joining the prompt reader task failed"))]
    JoinPromptReader { source: tokio::task::JoinError },

    #[snafu(display("flushing the prompt failed"))]
    FlushPrompt { source: std::io::Error },

    #[snafu(display("reading a prompt line failed"))]
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

    #[snafu(display("decoding the {action} response failed"))]
    DecodeResponse {
        source: reqwest::Error,
        action: &'static str,
    },

    #[snafu(display("reading the event stream failed"))]
    ReadEventStream {
        source: Box<dyn StdError + Send + Sync + 'static>,
    },

    #[snafu(display("decoding the stream event failed"))]
    DecodeStreamEvent { source: serde_json::Error },

    #[snafu(display("unable to determine a recent-session cache directory"))]
    MissingRecentSessionDirectory,

    #[snafu(display("reading the recent-session cache from {} failed", path.display()))]
    ReadRecentSessions {
        source: std::io::Error,
        path: PathBuf,
    },

    #[snafu(display("parsing the recent-session cache from {} failed", path.display()))]
    ParseRecentSessions {
        source: serde_json::Error,
        path: PathBuf,
    },

    #[snafu(display("creating the recent-session cache directory {} failed", path.display()))]
    CreateRecentSessionsDirectory {
        source: std::io::Error,
        path: PathBuf,
    },

    #[snafu(display("serializing the recent-session cache failed"))]
    SerializeRecentSessions { source: serde_json::Error },

    #[snafu(display("writing the recent-session cache to {} failed", path.display()))]
    WriteRecentSessions {
        source: std::io::Error,
        path: PathBuf,
    },
}

#[derive(Parser, Debug)]
#[command(name = "acp")]
#[command(about = "ACP Orchestrator CLI frontend")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    Chat(ChatArgs),
    Session(SessionArgs),
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

pub async fn run_with_args<I, T>(args: I) -> Result<()>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    init_tracing();

    let cli = Cli::parse_from(args);
    match cli.command {
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

async fn run_chat(args: ChatArgs) -> Result<()> {
    ensure!(args.new || args.session_id.is_some(), ChatModeRequiredSnafu);

    let client = Client::builder().build().context(BuildHttpClientSnafu)?;
    let server_url = require_server_url("chat", args.server_url.clone())?;

    let session = if args.new {
        create_session(&client, &server_url, &args.auth_token).await?
    } else {
        get_session(
            &client,
            &server_url,
            &args.auth_token,
            args.session_id
                .as_deref()
                .expect("session id checked before chat execution"),
        )
        .await?
    };

    record_recent_session(&RecentSessionEntry {
        session_id: session.id.clone(),
        server_url: server_url.clone(),
        last_used_at: Utc::now(),
    })?;

    println!("session: {}", session.id);
    println!("connected to backend: {server_url}");

    let events_url = format!("{server_url}/api/v1/sessions/{}/events", session.id);
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

        submit_prompt(&client, &server_url, &args.auth_token, &session.id, trimmed).await?;
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
            let server_url = require_server_url("closing a session", args.server_url)?;
            let client = Client::builder().build().context(BuildHttpClientSnafu)?;
            close_session(&client, &server_url, &args.auth_token, &args.session_id).await?;
            remove_recent_session(&args.session_id)?;
            println!("[status] session {} closed", args.session_id);
            Ok(())
        }
    }
}

fn require_server_url(command: &'static str, server_url: Option<String>) -> Result<String> {
    server_url.ok_or_else(|| MissingServerUrlSnafu { command }.build())
}

async fn handle_repl_command(command: &str) -> Result<bool> {
    match command {
        "/help" => {
            println!("/help");
            println!("/quit");
            println!("/cancel (planned)");
            println!("/approve <request-id> (planned)");
            println!("/deny <request-id> (planned)");
            Ok(false)
        }
        "/quit" => Ok(true),
        value if value.starts_with("/cancel") => {
            println!("[status] `/cancel` is planned.");
            Ok(false)
        }
        value if value.starts_with("/approve ") => {
            println!("[status] `/approve` is planned.");
            Ok(false)
        }
        value if value.starts_with("/deny ") => {
            println!("[status] `/deny` is planned.");
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

async fn create_session(
    client: &Client,
    base_url: &str,
    auth_token: &str,
) -> Result<SessionSnapshot> {
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
) -> Result<SessionSnapshot> {
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
