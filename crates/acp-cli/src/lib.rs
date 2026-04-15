use std::{error::Error as StdError, ffi::OsString, future::Future, path::PathBuf};

use acp_app_support::{build_http_client_for_url, init_tracing};
use acp_contracts::{
    CancelTurnResponse, CloseSessionResponse, ConversationMessage, CreateSessionResponse,
    ErrorResponse, MessageRole, PermissionDecision, PromptRequest, PromptResponse,
    ResolvePermissionRequest, ResolvePermissionResponse, SessionSnapshot, StreamEvent,
    StreamEventPayload,
};
use chrono::Utc;
use clap::{Args, Parser, Subcommand};
use reqwest::{Client, Response, StatusCode};
use snafu::prelude::*;

mod api;
mod events;
mod input;
mod recent_sessions;
mod repl_commands;
mod tui;

#[cfg(test)]
mod chat_tests;
#[cfg(test)]
mod tests;

use api::{close_session, create_session, ensure_success, get_session, get_session_history};
use events::{InitialSnapshotState, stream_events_to_stderr};
use input::{drive_repl, interactive_completion_enabled};
use recent_sessions::{
    RecentSessionEntry, load_recent_sessions, record_recent_session, remove_recent_session,
};

pub type Result<T, E = CliError> = std::result::Result<T, E>;

pub(crate) struct ChatSession {
    session: SessionSnapshot,
    resume_history: Vec<ConversationMessage>,
    resumed: bool,
}

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

    #[snafu(display("building the interactive line editor failed"))]
    BuildInteractiveEditor {
        source: rustyline::error::ReadlineError,
    },

    #[snafu(display("joining the interactive terminal UI task failed"))]
    JoinInteractiveUi { source: tokio::task::JoinError },

    #[snafu(display("joining the prompt reader task failed"))]
    JoinPromptReader { source: tokio::task::JoinError },

    #[snafu(display("flushing the prompt failed"))]
    FlushPrompt { source: std::io::Error },

    #[snafu(display("reading a prompt line failed"))]
    ReadPromptLine { source: std::io::Error },

    #[snafu(display("reading interactive input failed"))]
    ReadInteractivePrompt {
        source: rustyline::error::ReadlineError,
    },

    #[snafu(display("setting up the terminal UI failed"))]
    SetupTerminalUi { source: std::io::Error },

    #[snafu(display("drawing the terminal UI failed"))]
    DrawTerminalUi { source: std::io::Error },

    #[snafu(display("polling for terminal input failed"))]
    PollTerminalInput { source: std::io::Error },

    #[snafu(display("reading terminal input failed"))]
    ReadTerminalInput { source: std::io::Error },

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
    run_chat_with_ui(args, interactive_completion_enabled(), tui::run_chat_tui).await
}

async fn run_chat_with_handlers<RunUi, UiFuture, RunRepl, ReplFuture>(
    args: ChatArgs,
    interactive_ui: bool,
    run_ui: RunUi,
    run_repl: RunRepl,
) -> Result<()>
where
    RunUi: FnOnce(Client, String, String, ChatSession) -> UiFuture,
    UiFuture: Future<Output = Result<()>>,
    RunRepl: FnOnce(Client, String, String, String) -> ReplFuture,
    ReplFuture: Future<Output = Result<()>>,
{
    ensure!(args.new || args.session_id.is_some(), ChatModeRequiredSnafu);

    let server_url = require_server_url("chat", args.server_url.clone())?;
    let client = build_http_client_for_url(&server_url, None).context(BuildHttpClientSnafu)?;
    let chat_session = load_chat_session(&client, &server_url, &args).await?;
    let session_id = chat_session.session.id.clone();
    let recent_entry = RecentSessionEntry::new(&session_id, &server_url, Utc::now());
    record_recent_session(&recent_entry)?;
    if interactive_ui {
        return run_ui(client, server_url, args.auth_token, chat_session).await;
    }

    print_chat_banner(&chat_session.session.id, &server_url);
    print_chat_status(&chat_session, false);
    let initial_snapshot_state = render_resume_history(&chat_session);
    let event_task = spawn_event_task(
        &client,
        &server_url,
        &args.auth_token,
        &chat_session.session.id,
        initial_snapshot_state,
    );
    let repl_result = run_repl(client, server_url, args.auth_token, session_id).await;
    event_task.abort();
    repl_result
}

async fn run_chat_with_ui<RunUi, UiFuture>(
    args: ChatArgs,
    interactive_ui: bool,
    run_ui: RunUi,
) -> Result<()>
where
    RunUi: FnOnce(Client, String, String, ChatSession) -> UiFuture,
    UiFuture: Future<Output = Result<()>>,
{
    run_chat_with_handlers(
        args,
        interactive_ui,
        run_ui,
        |client, server_url, auth_token, session_id| async move {
            drive_repl(&client, &server_url, &auth_token, &session_id).await
        },
    )
    .await
}

async fn load_chat_session(
    client: &Client,
    server_url: &str,
    args: &ChatArgs,
) -> Result<ChatSession> {
    if args.new {
        return create_session(client, server_url, &args.auth_token)
            .await
            .map(|session| ChatSession {
                session,
                resume_history: Vec::new(),
                resumed: false,
            });
    }

    let session_id = args
        .session_id
        .as_deref()
        .expect("session id checked before chat execution");
    // Load the explicit history endpoint for transcript rendering, then fetch
    // the later session snapshot so pending permissions and SSE dedupe start
    // from the latest known state.
    let history = get_session_history(client, server_url, &args.auth_token, session_id).await?;
    let session = get_session(client, server_url, &args.auth_token, session_id).await?;

    Ok(ChatSession {
        session,
        resume_history: history.messages,
        resumed: true,
    })
}

fn render_resume_history(chat_session: &ChatSession) -> Option<InitialSnapshotState> {
    if !chat_session.resumed {
        return None;
    }

    let initial_snapshot_state = InitialSnapshotState::from_messages_and_permissions(
        &chat_session.resume_history,
        &chat_session.session.pending_permissions,
    );
    events::render_resume_state(
        &chat_session.resume_history,
        &chat_session.session.pending_permissions,
    );
    Some(initial_snapshot_state)
}

fn print_chat_banner(session_id: &str, server_url: &str) {
    println!("session: {session_id}");
    println!("connected to backend: {server_url}");
}

fn print_chat_status(chat_session: &ChatSession, interactive_completion: bool) {
    if chat_session.resumed {
        println!("[status] resumed existing session");
    } else {
        println!("[status] new session ready");
    }
    if interactive_completion {
        println!("[status] press TAB after `/` to view slash command candidates");
    }
    let pending_permissions = chat_session.session.pending_permissions.len();
    if pending_permissions > 0 {
        println!("[status] {pending_permissions} pending permission request(s) need attention");
    }
}

fn spawn_event_task(
    client: &Client,
    server_url: &str,
    auth_token: &str,
    session_id: &str,
    initial_snapshot_state: Option<InitialSnapshotState>,
) -> tokio::task::JoinHandle<()> {
    let events_url = format!("{server_url}/api/v1/sessions/{session_id}/events");
    tokio::spawn(stream_events_to_stderr(
        client.clone(),
        events_url,
        auth_token.to_string(),
        initial_snapshot_state,
    ))
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
