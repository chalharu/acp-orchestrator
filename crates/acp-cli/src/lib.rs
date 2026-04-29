use std::{error::Error as StdError, ffi::OsString, future::Future, io::IsTerminal, path::PathBuf};

use clap::{Args, Parser, Subcommand};
use reqwest::{Client, Response, StatusCode};
use snafu::prelude::*;

mod api;
pub mod contract_errors;
pub mod contract_messages;
pub mod contract_permissions;
pub mod contract_sessions;
pub mod contract_slash;
pub mod contract_stream;
pub mod contract_workspaces;
mod events;
mod input;
mod repl_commands;
pub mod support;
mod tui;

#[cfg(test)]
mod chat_tests;
#[cfg(test)]
mod tests;

use api::{
    close_session, create_workspace, create_workspace_session, ensure_success, get_session,
    get_session_history, list_sessions, list_workspaces,
};
use contract_errors::ErrorResponse;
use contract_messages::{ConversationMessage, MessageRole, PromptRequest, PromptResponse};
use contract_permissions::{
    PermissionDecision, ResolvePermissionRequest, ResolvePermissionResponse,
};
use contract_sessions::{CancelTurnResponse, CloseSessionResponse, SessionSnapshot, SessionStatus};
use contract_stream::{StreamEvent, StreamEventPayload};
use events::{InitialSnapshotState, stream_events_to_stderr};
use input::drive_repl;
use support::http::build_http_client_for_url;
use support::tracing::init_tracing;

pub type Result<T, E = CliError> = std::result::Result<T, E>;
const DEFAULT_AUTH_TOKEN: &str = "developer";

#[derive(Debug)]
pub(crate) struct ChatSession {
    session: SessionSnapshot,
    resume_history: Vec<ConversationMessage>,
    resumed: bool,
}

impl ChatSession {
    fn is_read_only(&self) -> bool {
        self.session.status == SessionStatus::Closed
    }
}

#[derive(Debug, Snafu)]
pub enum CliError {
    #[snafu(display("choose either `--new` or `--session <id>`"))]
    ChatModeRequired,

    #[snafu(display(
        "workspace selection required for new chat ({reason}); run `acp workspace list` or `acp workspace create <name> <upstream-url>`, then pass `acp chat --new --workspace <id>`"
    ))]
    WorkspaceSelectionRequired { reason: String },

    #[snafu(display(
        "{command} requires `--server-url` or ACP_SERVER_URL to point at a running backend"
    ))]
    MissingServerUrl { command: &'static str },

    #[snafu(display("building the HTTP client failed"))]
    BuildHttpClient { source: reqwest::Error },

    #[snafu(display("joining the interactive terminal UI task failed"))]
    JoinInteractiveUi { source: tokio::task::JoinError },

    #[snafu(display("joining the prompt reader task failed"))]
    JoinPromptReader { source: tokio::task::JoinError },

    #[snafu(display("flushing the prompt failed"))]
    FlushPrompt { source: std::io::Error },

    #[snafu(display("reading a prompt line failed"))]
    ReadPromptLine { source: std::io::Error },

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
    Workspace(WorkspaceArgs),
}

#[derive(Args, Debug)]
struct ChatArgs {
    #[arg(long)]
    new: bool,
    #[arg(long = "session")]
    session_id: Option<String>,
    #[arg(long = "workspace")]
    workspace_id: Option<String>,
    #[arg(long, env = "ACP_SERVER_URL")]
    server_url: Option<String>,
    #[arg(long, env = "ACP_AUTH_TOKEN")]
    auth_token: Option<String>,
}

#[derive(Args, Debug)]
struct SessionArgs {
    #[command(subcommand)]
    command: SessionCommand,
}

#[derive(Args, Debug)]
struct WorkspaceArgs {
    #[command(subcommand)]
    command: WorkspaceCommand,
}

#[derive(Subcommand, Debug)]
enum WorkspaceCommand {
    List(WorkspaceListArgs),
    Create(WorkspaceCreateArgs),
}

#[derive(Args, Debug)]
struct WorkspaceListArgs {
    #[arg(long, env = "ACP_SERVER_URL")]
    server_url: Option<String>,
    #[arg(long, env = "ACP_AUTH_TOKEN")]
    auth_token: Option<String>,
}

#[derive(Args, Debug)]
struct WorkspaceCreateArgs {
    name: String,
    upstream_url: String,
    #[arg(long, env = "ACP_SERVER_URL")]
    server_url: Option<String>,
    #[arg(long, env = "ACP_AUTH_TOKEN")]
    auth_token: Option<String>,
}

#[derive(Subcommand, Debug)]
enum SessionCommand {
    List(ListArgs),
    Close(CloseArgs),
}

#[derive(Args, Debug)]
struct ListArgs {
    #[arg(long, env = "ACP_SERVER_URL")]
    server_url: Option<String>,
    #[arg(long, env = "ACP_AUTH_TOKEN")]
    auth_token: Option<String>,
}

#[derive(Args, Debug)]
struct CloseArgs {
    session_id: String,
    #[arg(long, env = "ACP_SERVER_URL")]
    server_url: Option<String>,
    #[arg(long, env = "ACP_AUTH_TOKEN")]
    auth_token: Option<String>,
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
        Command::Workspace(args) => run_workspace(args).await,
    }
}

async fn run_chat(args: ChatArgs) -> Result<()> {
    run_chat_with_ui(args, interactive_terminal_available(), tui::run_chat_tui).await
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
    let auth_token = resolved_auth_token(args.auth_token.clone());
    let client = build_http_client_for_url(&server_url, None).context(BuildHttpClientSnafu)?;
    let chat_session = load_chat_session(
        &client,
        &server_url,
        args.new,
        args.session_id.as_deref(),
        args.workspace_id.as_deref(),
        &auth_token,
    )
    .await?;
    let session_id = chat_session.session.id.clone();
    if chat_session.is_read_only() {
        print_chat_banner(&chat_session.session.id, &server_url);
        print_chat_status(&chat_session, false);
        let _ = render_resume_history(&chat_session);
        return Ok(());
    }
    if interactive_ui {
        return run_ui(client, server_url, auth_token, chat_session).await;
    }

    print_chat_banner(&chat_session.session.id, &server_url);
    print_chat_status(&chat_session, false);
    let initial_snapshot_state = render_resume_history(&chat_session);
    let event_task = spawn_event_task(
        &client,
        &server_url,
        &auth_token,
        &chat_session.session.id,
        initial_snapshot_state,
    );
    let repl_result = run_repl(client, server_url, auth_token, session_id).await;
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

fn interactive_terminal_available() -> bool {
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

async fn load_chat_session(
    client: &Client,
    server_url: &str,
    new_session: bool,
    session_id: Option<&str>,
    workspace_id: Option<&str>,
    auth_token: &str,
) -> Result<ChatSession> {
    if new_session {
        let workspace_id =
            resolve_workspace_for_new_chat(client, server_url, auth_token, workspace_id).await?;
        return create_workspace_session(client, server_url, auth_token, &workspace_id)
            .await
            .map(|session| ChatSession {
                session,
                resume_history: Vec::new(),
                resumed: false,
            });
    }

    let session_id = session_id.expect("session id checked before chat execution");
    let session = get_session(client, server_url, auth_token, session_id).await?;
    let resume_history = match get_session_history(client, server_url, auth_token, session_id).await
    {
        Ok(history) => history.messages,
        Err(error) if is_session_not_found(&error) => session.messages.clone(),
        Err(error) => return Err(error),
    };

    Ok(ChatSession {
        session,
        resume_history,
        resumed: true,
    })
}

async fn resolve_workspace_for_new_chat(
    client: &Client,
    server_url: &str,
    auth_token: &str,
    workspace_id: Option<&str>,
) -> Result<String> {
    if let Some(workspace_id) = workspace_id.filter(|id| !id.trim().is_empty()) {
        return Ok(workspace_id.to_string());
    }

    let workspaces = list_workspaces(client, server_url, auth_token)
        .await?
        .workspaces;
    match workspaces.as_slice() {
        [workspace] => Ok(workspace.workspace_id.clone()),
        [] => Err(CliError::WorkspaceSelectionRequired {
            reason: "no workspaces exist".to_string(),
        }),
        _ => Err(CliError::WorkspaceSelectionRequired {
            reason: "multiple workspaces exist".to_string(),
        }),
    }
}

fn resolved_auth_token(auth_token: Option<String>) -> String {
    auth_token.unwrap_or_else(|| DEFAULT_AUTH_TOKEN.to_string())
}

fn is_session_not_found(error: &CliError) -> bool {
    matches!(
        error,
        CliError::HttpStatus {
            status,
            message,
            ..
        } if *status == StatusCode::NOT_FOUND && message == "session not found"
    )
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
    if chat_session.is_read_only() {
        println!("[status] opened closed session as read-only transcript");
    } else if chat_session.resumed {
        println!("[status] resumed existing session");
    } else {
        println!("[status] new session ready");
    }
    if interactive_completion && !chat_session.is_read_only() {
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
        SessionCommand::List(args) => {
            let server_url = require_server_url("listing sessions", args.server_url)?;
            let auth_token = resolved_auth_token(args.auth_token);
            let client =
                build_http_client_for_url(&server_url, None).context(BuildHttpClientSnafu)?;
            let sessions = list_sessions(&client, &server_url, &auth_token).await?;
            if sessions.sessions.is_empty() {
                println!("no sessions found for the current owner");
                return Ok(());
            }

            for session in sessions.sessions {
                println!(
                    "{}\t{}\t{}",
                    session.id,
                    session_status_label(&session.status),
                    session.last_activity_at.to_rfc3339()
                );
            }

            Ok(())
        }
        SessionCommand::Close(args) => {
            let server_url = require_server_url("closing a session", args.server_url)?;
            let auth_token = resolved_auth_token(args.auth_token);
            let client =
                build_http_client_for_url(&server_url, None).context(BuildHttpClientSnafu)?;
            close_session(&client, &server_url, &auth_token, &args.session_id).await?;
            println!("[status] session {} closed", args.session_id);
            Ok(())
        }
    }
}

async fn run_workspace(args: WorkspaceArgs) -> Result<()> {
    match args.command {
        WorkspaceCommand::List(args) => {
            let server_url = require_server_url("listing workspaces", args.server_url)?;
            let auth_token = resolved_auth_token(args.auth_token);
            let client =
                build_http_client_for_url(&server_url, None).context(BuildHttpClientSnafu)?;
            let workspaces = list_workspaces(&client, &server_url, &auth_token).await?;
            if workspaces.workspaces.is_empty() {
                println!("no workspaces found for the current owner");
                return Ok(());
            }
            for workspace in workspaces.workspaces {
                println!(
                    "{}\t{}\t{}",
                    workspace.workspace_id,
                    workspace.name,
                    workspace.upstream_url.unwrap_or_else(|| "-".to_string())
                );
            }
            Ok(())
        }
        WorkspaceCommand::Create(args) => {
            let server_url = require_server_url("creating a workspace", args.server_url)?;
            let auth_token = resolved_auth_token(args.auth_token);
            let client =
                build_http_client_for_url(&server_url, None).context(BuildHttpClientSnafu)?;
            let workspace = create_workspace(
                &client,
                &server_url,
                &auth_token,
                &args.name,
                &args.upstream_url,
            )
            .await?;
            println!(
                "{}\t{}\t{}",
                workspace.workspace_id,
                workspace.name,
                workspace.upstream_url.unwrap_or_else(|| "-".to_string())
            );
            Ok(())
        }
    }
}

fn session_status_label(status: &SessionStatus) -> &'static str {
    match status {
        SessionStatus::Active => "active",
        SessionStatus::Closed => "closed",
    }
}

fn require_server_url(command: &'static str, server_url: Option<String>) -> Result<String> {
    server_url.ok_or_else(|| MissingServerUrlSnafu { command }.build())
}
