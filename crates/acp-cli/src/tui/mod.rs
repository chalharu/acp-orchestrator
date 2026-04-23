#[cfg(test)]
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use reqwest::Client;
use snafu::ResultExt;
use tokio::{runtime::Handle, sync::mpsc};

use crate::contract_permissions::PermissionRequest;
use crate::contract_slash::CompletionCandidate;
use crate::{
    ChatSession, JoinInteractiveUiSnafu, Result,
    events::{InitialSnapshotState, StreamUpdate, stream_updates},
    repl_commands::load_command_catalog,
};

mod app;
mod input;
mod render;
mod runtime;

#[cfg(test)]
mod mod_tests;
#[cfg(test)]
mod tests;

const SLASH_COMPLETION_TIMEOUT: Duration = Duration::from_secs(2);
type TerminalUiRunner = fn(Handle, runtime::UiRunState) -> Result<()>;

#[cfg(test)]
static TERMINAL_UI_RUNNER_OVERRIDE: OnceLock<Mutex<Option<TerminalUiRunner>>> = OnceLock::new();

#[derive(Debug)]
enum TuiEvent {
    Stream(StreamUpdate),
    StreamEnded(String),
    PendingPermissionsRefreshed(std::result::Result<Vec<PermissionRequest>, String>),
}

struct StartupState {
    command_catalog: Vec<CompletionCandidate>,
    startup_statuses: Vec<String>,
}

pub(super) async fn run_chat_tui(
    client: Client,
    server_url: String,
    auth_token: String,
    chat_session: ChatSession,
) -> Result<()> {
    run_chat_tui_with_runner(
        client,
        server_url,
        auth_token,
        chat_session,
        terminal_ui_runner(),
    )
    .await
}

async fn run_chat_tui_with_runner<RunUi>(
    client: Client,
    server_url: String,
    auth_token: String,
    chat_session: ChatSession,
    run_ui: RunUi,
) -> Result<()>
where
    RunUi: FnOnce(Handle, runtime::UiRunState) -> Result<()> + Send + 'static,
{
    let initial_snapshot_state = chat_session.resumed.then(|| {
        InitialSnapshotState::from_messages_and_permissions(
            &chat_session.resume_history,
            &chat_session.session.pending_permissions,
        )
    });
    let (event_tx, event_rx) = mpsc::unbounded_channel();
    let stream_task = spawn_stream_task(
        client.clone(),
        server_url.clone(),
        auth_token.clone(),
        &chat_session,
        initial_snapshot_state,
        event_tx.clone(),
    );
    let startup = prepare_startup_state(&client, &server_url, &auth_token, &chat_session).await;

    let runtime_handle = Handle::current();
    let ui_result = tokio::task::spawn_blocking(move || {
        run_ui(
            runtime_handle,
            runtime::UiRunState::new(
                client,
                server_url,
                auth_token,
                chat_session,
                startup.command_catalog,
                startup.startup_statuses,
                runtime::UiEventChannel {
                    tx: event_tx,
                    rx: event_rx,
                },
            ),
        )
    })
    .await;
    stream_task.abort();
    ui_result.context(JoinInteractiveUiSnafu)?
}

fn terminal_ui_runner() -> TerminalUiRunner {
    #[cfg(test)]
    {
        if let Some(runner) = *TERMINAL_UI_RUNNER_OVERRIDE
            .get_or_init(|| Mutex::new(None))
            .lock()
            .expect("terminal ui runner override should not poison")
        {
            return runner;
        }
    }
    runtime::run_terminal_ui
}

#[cfg(test)]
fn set_terminal_ui_runner_override(runner: Option<TerminalUiRunner>) {
    *TERMINAL_UI_RUNNER_OVERRIDE
        .get_or_init(|| Mutex::new(None))
        .lock()
        .expect("terminal ui runner override should not poison") = runner;
}

async fn prepare_startup_state(
    client: &Client,
    server_url: &str,
    auth_token: &str,
    chat_session: &ChatSession,
) -> StartupState {
    prepare_startup_state_with_timeout(
        client,
        server_url,
        auth_token,
        chat_session,
        SLASH_COMPLETION_TIMEOUT,
    )
    .await
}

async fn prepare_startup_state_with_timeout(
    client: &Client,
    server_url: &str,
    auth_token: &str,
    chat_session: &ChatSession,
    timeout: Duration,
) -> StartupState {
    let mut startup_statuses = Vec::new();
    let command_catalog = match tokio::time::timeout(
        timeout,
        load_command_catalog(client, server_url, auth_token, &chat_session.session.id),
    )
    .await
    {
        Ok(Ok(command_catalog)) => command_catalog,
        Ok(Err(error)) => {
            startup_statuses.push(error.to_string());
            Vec::new()
        }
        Err(_) => {
            startup_statuses.push("slash command catalog timed out".to_string());
            Vec::new()
        }
    };

    StartupState {
        command_catalog,
        startup_statuses,
    }
}

fn spawn_stream_task(
    client: Client,
    server_url: String,
    auth_token: String,
    chat_session: &ChatSession,
    initial_snapshot_state: Option<InitialSnapshotState>,
    event_tx: mpsc::UnboundedSender<TuiEvent>,
) -> tokio::task::JoinHandle<()> {
    let events_url = format!(
        "{server_url}/api/v1/sessions/{}/events",
        chat_session.session.id
    );
    tokio::spawn(async move {
        let result = stream_updates(
            client,
            events_url,
            auth_token,
            initial_snapshot_state,
            |update| {
                let _ = event_tx.send(TuiEvent::Stream(update));
            },
        )
        .await;
        let message = result.err().map_or_else(
            || "event stream ended".to_string(),
            |error| format!("event stream ended: {error}"),
        );
        let _ = event_tx.send(TuiEvent::StreamEnded(message));
    })
}
