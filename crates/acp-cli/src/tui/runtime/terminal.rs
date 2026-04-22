use std::{io, time::Duration};

use crossterm::{
    event::{self, Event},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use tokio::runtime::Handle;

use super::{UiRunState, event_loop};
use crate::{PollTerminalInputSnafu, ReadTerminalInputSnafu, Result, SetupTerminalUiSnafu};
use snafu::ResultExt;

const UI_POLL_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Default)]
struct TerminalSetupGuard {
    raw_mode_enabled: bool,
    alternate_screen_enabled: bool,
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

pub(crate) fn run_terminal_ui(runtime_handle: Handle, state: UiRunState) -> Result<()> {
    let (mut terminal, mut setup_guard) = setup_terminal()?;
    let result = event_loop(&mut terminal, runtime_handle, state);
    let cleanup_result = restore_terminal(&mut terminal, &mut setup_guard);
    cleanup_result?;
    result
}

fn setup_terminal() -> Result<(Terminal<CrosstermBackend<io::Stdout>>, TerminalSetupGuard)> {
    let mut guard = TerminalSetupGuard::default();
    enable_raw_mode().context(SetupTerminalUiSnafu)?;
    guard.raw_mode_enabled = true;

    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).context(SetupTerminalUiSnafu)?;
    guard.alternate_screen_enabled = true;

    let terminal = Terminal::new(CrosstermBackend::new(stdout)).context(SetupTerminalUiSnafu)?;
    Ok((terminal, guard))
}

fn restore_terminal(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    guard: &mut TerminalSetupGuard,
) -> Result<()> {
    if guard.raw_mode_enabled {
        disable_raw_mode().context(SetupTerminalUiSnafu)?;
        guard.raw_mode_enabled = false;
    }
    if guard.alternate_screen_enabled {
        execute!(terminal.backend_mut(), LeaveAlternateScreen).context(SetupTerminalUiSnafu)?;
        guard.alternate_screen_enabled = false;
    }
    terminal.show_cursor().context(SetupTerminalUiSnafu)
}

pub(super) fn read_terminal_event() -> Result<Option<Event>> {
    if !event::poll(UI_POLL_INTERVAL).context(PollTerminalInputSnafu)? {
        return Ok(None);
    }
    event::read().context(ReadTerminalInputSnafu).map(Some)
}
