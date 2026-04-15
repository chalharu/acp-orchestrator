use std::{io, time::Duration};

use acp_contracts::{CompletionCandidate, MessageRole};
use crossterm::{
    event::{self, Event},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use reqwest::Client;
use snafu::ResultExt;
use tokio::{runtime::Handle, sync::mpsc};

use super::{
    TuiEvent,
    app::ChatApp,
    input::{UiContext, handle_terminal_event as handle_input_event},
    render,
};
use crate::{
    ChatSession, DrawTerminalUiSnafu, PollTerminalInputSnafu, ReadTerminalInputSnafu, Result,
    SetupTerminalUiSnafu,
};

const UI_POLL_INTERVAL: Duration = Duration::from_millis(50);

pub(super) struct UiRunState {
    client: Client,
    server_url: String,
    auth_token: String,
    chat_session: ChatSession,
    command_catalog: Vec<CompletionCandidate>,
    startup_statuses: Vec<String>,
    event_tx: mpsc::UnboundedSender<TuiEvent>,
    event_rx: mpsc::UnboundedReceiver<TuiEvent>,
}

pub(super) struct UiEventChannel {
    pub(super) tx: mpsc::UnboundedSender<TuiEvent>,
    pub(super) rx: mpsc::UnboundedReceiver<TuiEvent>,
}

impl UiRunState {
    pub(super) fn new(
        client: Client,
        server_url: String,
        auth_token: String,
        chat_session: ChatSession,
        command_catalog: Vec<CompletionCandidate>,
        startup_statuses: Vec<String>,
        events: UiEventChannel,
    ) -> Self {
        Self {
            client,
            server_url,
            auth_token,
            chat_session,
            command_catalog,
            startup_statuses,
            event_tx: events.tx,
            event_rx: events.rx,
        }
    }
}

#[derive(Default)]
struct PendingPermissionRefreshState {
    in_flight: bool,
    queued: bool,
}

#[derive(Default)]
struct TerminalSetupGuard {
    raw_mode_enabled: bool,
    alternate_screen_enabled: bool,
}

impl TerminalSetupGuard {
    fn disarm(&mut self) {
        self.raw_mode_enabled = false;
        self.alternate_screen_enabled = false;
    }
}

impl Drop for TerminalSetupGuard {
    fn drop(&mut self) {
        if self.raw_mode_enabled {
            let _ = disable_raw_mode();
        }
        if self.alternate_screen_enabled {
            let mut stdout = io::stdout();
            let _ = execute!(stdout, LeaveAlternateScreen);
        }
    }
}

pub(super) fn run_terminal_ui(runtime_handle: Handle, state: UiRunState) -> Result<()> {
    let mut terminal = setup_terminal()?;
    let result = event_loop(&mut terminal, runtime_handle, state);
    let cleanup_result = restore_terminal(&mut terminal);
    cleanup_result?;
    result
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>> {
    let mut guard = TerminalSetupGuard::default();
    enable_raw_mode().context(SetupTerminalUiSnafu)?;
    guard.raw_mode_enabled = true;

    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).context(SetupTerminalUiSnafu)?;
    guard.alternate_screen_enabled = true;

    let terminal = Terminal::new(CrosstermBackend::new(stdout)).context(SetupTerminalUiSnafu)?;
    guard.disarm();
    Ok(terminal)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    disable_raw_mode().context(SetupTerminalUiSnafu)?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen).context(SetupTerminalUiSnafu)?;
    terminal.show_cursor().context(SetupTerminalUiSnafu)
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    runtime_handle: Handle,
    state: UiRunState,
) -> Result<()> {
    let UiRunState {
        client,
        server_url,
        auth_token,
        chat_session,
        command_catalog,
        startup_statuses,
        event_tx,
        mut event_rx,
    } = state;

    let mut app = build_chat_app(
        &chat_session,
        &server_url,
        command_catalog,
        startup_statuses,
    );
    let context = UiContext {
        runtime_handle: &runtime_handle,
        client: &client,
        server_url: &server_url,
        auth_token: &auth_token,
        session_id: &chat_session.session.id,
    };
    let mut refresh_state = PendingPermissionRefreshState::default();

    loop {
        draw_app(
            terminal,
            &context,
            &event_tx,
            &mut event_rx,
            &mut app,
            &mut refresh_state,
        )?;
        if app.should_quit() {
            return Ok(());
        }

        if let Some(terminal_event) = read_terminal_event()? {
            let terminal_size = terminal.size().context(DrawTerminalUiSnafu)?;
            handle_input_event(terminal_size, &context, &mut app, terminal_event)?;
        }
    }
}

fn build_chat_app(
    chat_session: &ChatSession,
    server_url: &str,
    command_catalog: Vec<CompletionCandidate>,
    startup_statuses: Vec<String>,
) -> ChatApp {
    let mut app = ChatApp::new(
        &chat_session.session.id,
        server_url,
        chat_session.resumed,
        &chat_session.resume_history,
        &chat_session.session.pending_permissions,
        command_catalog,
    );
    for status in startup_statuses {
        app.push_status(status);
    }
    app
}

fn draw_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    context: &UiContext<'_>,
    event_tx: &mpsc::UnboundedSender<TuiEvent>,
    event_rx: &mut mpsc::UnboundedReceiver<TuiEvent>,
    app: &mut ChatApp,
    refresh_state: &mut PendingPermissionRefreshState,
) -> Result<()> {
    drain_events(event_rx, app, refresh_state);
    launch_pending_permission_refresh(context, event_tx, refresh_state);
    terminal
        .draw(|frame| render::render(frame, app))
        .context(DrawTerminalUiSnafu)?;
    Ok(())
}

fn read_terminal_event() -> Result<Option<Event>> {
    if !event::poll(UI_POLL_INTERVAL).context(PollTerminalInputSnafu)? {
        return Ok(None);
    }
    event::read().context(ReadTerminalInputSnafu).map(Some)
}

fn drain_events(
    event_rx: &mut mpsc::UnboundedReceiver<TuiEvent>,
    app: &mut ChatApp,
    refresh_state: &mut PendingPermissionRefreshState,
) {
    let mut queue_pending_permission_refresh = false;
    loop {
        match event_rx.try_recv() {
            Ok(TuiEvent::Stream(update)) => {
                queue_pending_permission_refresh |=
                    should_refresh_pending_permissions(app, &update);
                app.apply_stream_update(update);
            }
            Ok(TuiEvent::StreamEnded(message)) => app.set_connection_lost(message),
            Ok(TuiEvent::PendingPermissionsRefreshed(result)) => {
                refresh_state.in_flight = false;
                match result {
                    Ok(pending_permissions) => app.replace_pending_permissions(pending_permissions),
                    Err(error) => app.push_status(error),
                }
            }
            Err(mpsc::error::TryRecvError::Empty)
            | Err(mpsc::error::TryRecvError::Disconnected) => {
                if queue_pending_permission_refresh {
                    refresh_state.queued = true;
                }
                return;
            }
        }
    }
}

fn should_refresh_pending_permissions(app: &ChatApp, update: &crate::events::StreamUpdate) -> bool {
    if app.pending_permissions().is_empty() {
        return false;
    }

    match update {
        crate::events::StreamUpdate::Status(_)
        | crate::events::StreamUpdate::SessionClosed { .. } => true,
        crate::events::StreamUpdate::ConversationMessage(message) => {
            matches!(message.role, MessageRole::Assistant)
        }
        crate::events::StreamUpdate::PermissionRequested(_) => false,
    }
}

fn launch_pending_permission_refresh(
    context: &UiContext<'_>,
    event_tx: &mpsc::UnboundedSender<TuiEvent>,
    refresh_state: &mut PendingPermissionRefreshState,
) {
    if refresh_state.in_flight || !refresh_state.queued {
        return;
    }

    refresh_state.in_flight = true;
    refresh_state.queued = false;

    let client = context.client.clone();
    let server_url = context.server_url.to_string();
    let auth_token = context.auth_token.to_string();
    let session_id = context.session_id.to_string();
    let event_tx = event_tx.clone();
    context.runtime_handle.spawn(async move {
        let result = crate::api::get_session(&client, &server_url, &auth_token, &session_id)
            .await
            .map(|session| session.pending_permissions)
            .map_err(|error| error.to_string());
        let _ = event_tx.send(TuiEvent::PendingPermissionsRefreshed(result));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use acp_contracts::{CreateSessionResponse, PermissionRequest, SessionSnapshot, SessionStatus};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    fn pending_permission_app() -> ChatApp {
        ChatApp::new(
            "s_test",
            "http://127.0.0.1:8080",
            false,
            &[],
            &[PermissionRequest {
                request_id: "req_1".to_string(),
                summary: "read_text_file README.md".to_string(),
            }],
            vec![],
        )
    }

    #[test]
    fn drain_events_requests_pending_permission_refresh_after_status_updates() {
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let mut app = pending_permission_app();
        let mut refresh_state = PendingPermissionRefreshState::default();
        event_tx
            .send(TuiEvent::Stream(crate::events::StreamUpdate::Status(
                "turn cancelled".to_string(),
            )))
            .expect("status updates should queue");

        drain_events(&mut event_rx, &mut app, &mut refresh_state);
        assert!(refresh_state.queued);
        assert!(
            app.status_entries()
                .iter()
                .any(|status| status == "turn cancelled")
        );
    }

    #[test]
    fn drain_events_applies_refreshed_pending_permissions() {
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let mut app = pending_permission_app();
        let mut refresh_state = PendingPermissionRefreshState {
            in_flight: true,
            queued: false,
        };
        event_tx
            .send(TuiEvent::PendingPermissionsRefreshed(Ok(Vec::new())))
            .expect("refreshed permissions should queue");

        drain_events(&mut event_rx, &mut app, &mut refresh_state);

        assert!(app.pending_permissions().is_empty());
        assert!(!refresh_state.in_flight);
    }

    #[tokio::test]
    async fn launch_pending_permission_refresh_fetches_latest_permissions() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("server should bind");
        let address = listener
            .local_addr()
            .expect("server address should be readable");
        let payload = serde_json::to_vec(&CreateSessionResponse {
            session: SessionSnapshot {
                id: "s_test".to_string(),
                status: SessionStatus::Active,
                latest_sequence: 2,
                messages: Vec::new(),
                pending_permissions: Vec::new(),
            },
        })
        .expect("session payload should serialize");
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("server should accept");
            let mut buffer = [0u8; 1024];
            let _ = stream.read(&mut buffer).await;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                payload.len()
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("response headers should write");
            stream
                .write_all(&payload)
                .await
                .expect("response body should write");
        });

        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let client = Client::builder().build().expect("client should build");
        let runtime_handle = Handle::current();
        let server_url = format!("http://{address}");
        let mut refresh_state = PendingPermissionRefreshState {
            in_flight: false,
            queued: true,
        };
        let context = UiContext {
            runtime_handle: &runtime_handle,
            client: &client,
            server_url: &server_url,
            auth_token: "developer",
            session_id: "s_test",
        };

        launch_pending_permission_refresh(&context, &event_tx, &mut refresh_state);
        assert!(refresh_state.in_flight);
        assert!(!refresh_state.queued);
        match event_rx.recv().await.expect("refresh result should arrive") {
            TuiEvent::PendingPermissionsRefreshed(Ok(pending_permissions)) => {
                assert!(pending_permissions.is_empty());
            }
            other => panic!("unexpected refresh event: {other:?}"),
        }
    }
}
