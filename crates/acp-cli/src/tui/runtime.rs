use std::{io, time::Duration};

use acp_contracts::CompletionCandidate;
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
    event_rx: mpsc::UnboundedReceiver<TuiEvent>,
}

impl UiRunState {
    pub(super) fn new(
        client: Client,
        server_url: String,
        auth_token: String,
        chat_session: ChatSession,
        command_catalog: Vec<CompletionCandidate>,
        startup_statuses: Vec<String>,
        event_rx: mpsc::UnboundedReceiver<TuiEvent>,
    ) -> Self {
        Self {
            client,
            server_url,
            auth_token,
            chat_session,
            command_catalog,
            startup_statuses,
            event_rx,
        }
    }
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

    loop {
        draw_app(terminal, &mut event_rx, &mut app)?;
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
    event_rx: &mut mpsc::UnboundedReceiver<TuiEvent>,
    app: &mut ChatApp,
) -> Result<()> {
    drain_events(event_rx, app);
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

fn drain_events(event_rx: &mut mpsc::UnboundedReceiver<TuiEvent>, app: &mut ChatApp) {
    loop {
        match event_rx.try_recv() {
            Ok(TuiEvent::Stream(update)) => app.apply_stream_update(update),
            Ok(TuiEvent::StreamEnded(message)) => app.set_connection_lost(message),
            Err(mpsc::error::TryRecvError::Empty)
            | Err(mpsc::error::TryRecvError::Disconnected) => return,
        }
    }
}
