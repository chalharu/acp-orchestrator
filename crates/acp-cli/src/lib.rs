use std::{
    error::Error as StdError,
    ffi::OsString,
    io::{self, Write},
    path::PathBuf,
};

use acp_app_support::{build_http_client_for_url, init_tracing};
use acp_contracts::{
    CancelTurnResponse, CloseSessionResponse, CreateSessionResponse, ErrorResponse, MessageRole,
    PermissionDecision, PromptRequest, PromptResponse, ResolvePermissionRequest,
    ResolvePermissionResponse, SessionSnapshot, StreamEvent, StreamEventPayload,
};
use chrono::Utc;
use clap::{Args, Parser, Subcommand};
use reqwest::{Client, Response, StatusCode};
use snafu::prelude::*;

mod api;
mod events;
mod recent_sessions;
mod repl_commands;

#[cfg(test)]
mod tests;

use api::{close_session, create_session, ensure_success, get_session, submit_prompt};
use events::stream_events_to_stderr;
use recent_sessions::{
    RecentSessionEntry, load_recent_sessions, record_recent_session, remove_recent_session,
};
use repl_commands::handle_repl_command;

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

async fn run_chat(args: ChatArgs) -> Result<()> {
    ensure!(args.new || args.session_id.is_some(), ChatModeRequiredSnafu);

    let server_url = require_server_url("chat", args.server_url.clone())?;
    let client = build_http_client_for_url(&server_url, None).context(BuildHttpClientSnafu)?;
    let session = load_chat_session(&client, &server_url, &args).await?;
    record_recent_session(&RecentSessionEntry::new(
        &session.id,
        &server_url,
        Utc::now(),
    ))?;
    print_chat_banner(&session.id, &server_url);

    let event_task = spawn_event_task(&client, &server_url, &args.auth_token, &session.id);
    drive_repl(&client, &server_url, &args.auth_token, &session.id).await?;
    event_task.abort();
    Ok(())
}

async fn load_chat_session(
    client: &Client,
    server_url: &str,
    args: &ChatArgs,
) -> Result<SessionSnapshot> {
    if args.new {
        return create_session(client, server_url, &args.auth_token).await;
    }

    get_session(
        client,
        server_url,
        &args.auth_token,
        args.session_id
            .as_deref()
            .expect("session id checked before chat execution"),
    )
    .await
}

fn print_chat_banner(session_id: &str, server_url: &str) {
    println!("session: {session_id}");
    println!("connected to backend: {server_url}");
}

fn spawn_event_task(
    client: &Client,
    server_url: &str,
    auth_token: &str,
    session_id: &str,
) -> tokio::task::JoinHandle<()> {
    let events_url = format!("{server_url}/api/v1/sessions/{session_id}/events");
    tokio::spawn(stream_events_to_stderr(
        client.clone(),
        events_url,
        auth_token.to_string(),
    ))
}

async fn drive_repl(
    client: &Client,
    base_url: &str,
    auth_token: &str,
    session_id: &str,
) -> Result<()> {
    loop {
        let Some(line) = read_prompt_line().await? else {
            return Ok(());
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with('/') {
            if handle_repl_command(trimmed, client, base_url, auth_token, session_id).await? {
                return Ok(());
            }
            continue;
        }
        submit_prompt(client, base_url, auth_token, session_id, trimmed).await?;
    }
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
            let client =
                build_http_client_for_url(&server_url, None).context(BuildHttpClientSnafu)?;
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
