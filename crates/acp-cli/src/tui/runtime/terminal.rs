use std::{io, time::Duration};

#[cfg(test)]
use std::{
    collections::VecDeque,
    sync::{Mutex, OnceLock},
};

use crossterm::event::Event;
#[cfg(not(test))]
use crossterm::{
    event, execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
#[cfg(test)]
use ratatui::{TerminalOptions, Viewport, layout::Rect};
use tokio::runtime::Handle;

use super::{UiRunState, event_loop};
use crate::{PollTerminalInputSnafu, ReadTerminalInputSnafu, Result, SetupTerminalUiSnafu};
use snafu::ResultExt;

const UI_POLL_INTERVAL: Duration = Duration::from_millis(50);

#[cfg(test)]
type TerminalEventLoop =
    fn(&mut Terminal<CrosstermBackend<io::Stdout>>, Handle, UiRunState) -> Result<()>;

#[derive(Default)]
struct TerminalSetupGuard {
    raw_mode_enabled: bool,
    alternate_screen_enabled: bool,
}

#[cfg(test)]
#[derive(Default)]
struct TerminalTestState {
    enable_raw_mode_calls: usize,
    disable_raw_mode_calls: usize,
    enter_alternate_screen_calls: usize,
    leave_alternate_screen_calls: usize,
    show_cursor_calls: usize,
    poll_results: VecDeque<io::Result<bool>>,
    read_results: VecDeque<io::Result<Event>>,
}

#[cfg(test)]
fn terminal_test_state() -> &'static Mutex<TerminalTestState> {
    static STATE: OnceLock<Mutex<TerminalTestState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(TerminalTestState::default()))
}

#[cfg(test)]
fn terminal_event_loop_override() -> &'static Mutex<Option<TerminalEventLoop>> {
    static OVERRIDE: OnceLock<Mutex<Option<TerminalEventLoop>>> = OnceLock::new();
    OVERRIDE.get_or_init(|| Mutex::new(None))
}

impl Drop for TerminalSetupGuard {
    fn drop(&mut self) {
        if self.raw_mode_enabled {
            let _ = disable_terminal_raw_mode();
        }
        if self.alternate_screen_enabled {
            let mut stdout = io::stdout();
            let _ = leave_terminal_alternate_screen_stdout(&mut stdout);
        }
    }
}

#[rustfmt::skip]
fn run_event_loop(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, runtime_handle: Handle, state: UiRunState) -> Result<()> { #[cfg(test)] if let Some(override_loop) = *terminal_event_loop_override().lock().expect("terminal event loop override mutex should not be poisoned") { return override_loop(terminal, runtime_handle, state); } event_loop(terminal, runtime_handle, state) }

#[rustfmt::skip]
fn enable_terminal_raw_mode() -> io::Result<()> { #[cfg(test)] { terminal_test_state().lock().expect("terminal test state mutex should not be poisoned").enable_raw_mode_calls += 1; Ok(()) } #[cfg(not(test))] { enable_raw_mode() } }

#[rustfmt::skip]
fn disable_terminal_raw_mode() -> io::Result<()> { #[cfg(test)] { terminal_test_state().lock().expect("terminal test state mutex should not be poisoned").disable_raw_mode_calls += 1; Ok(()) } #[cfg(not(test))] { disable_raw_mode() } }

#[rustfmt::skip]
fn enter_terminal_alternate_screen(stdout: &mut io::Stdout) -> io::Result<()> { #[cfg(test)] { let _ = stdout; terminal_test_state().lock().expect("terminal test state mutex should not be poisoned").enter_alternate_screen_calls += 1; Ok(()) } #[cfg(not(test))] { execute!(stdout, EnterAlternateScreen) } }

#[rustfmt::skip]
fn leave_terminal_alternate_screen_stdout(stdout: &mut io::Stdout) -> io::Result<()> { #[cfg(test)] { let _ = stdout; terminal_test_state().lock().expect("terminal test state mutex should not be poisoned").leave_alternate_screen_calls += 1; Ok(()) } #[cfg(not(test))] { execute!(stdout, LeaveAlternateScreen) } }

#[rustfmt::skip]
fn leave_terminal_alternate_screen_backend(backend: &mut CrosstermBackend<io::Stdout>) -> io::Result<()> { #[cfg(test)] { let _ = backend; terminal_test_state().lock().expect("terminal test state mutex should not be poisoned").leave_alternate_screen_calls += 1; Ok(()) } #[cfg(not(test))] { execute!(backend, LeaveAlternateScreen) } }

#[rustfmt::skip]
fn show_terminal_cursor(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<()> { #[cfg(test)] { let _ = terminal; terminal_test_state().lock().expect("terminal test state mutex should not be poisoned").show_cursor_calls += 1; Ok(()) } #[cfg(not(test))] { terminal.show_cursor() } }

#[rustfmt::skip]
fn build_terminal(stdout: io::Stdout) -> io::Result<Terminal<CrosstermBackend<io::Stdout>>> { #[cfg(test)] { Terminal::with_options(CrosstermBackend::new(stdout), TerminalOptions { viewport: Viewport::Fixed(Rect::new(0, 0, 80, 24)) }) } #[cfg(not(test))] { Terminal::new(CrosstermBackend::new(stdout)) } }

#[rustfmt::skip]
fn poll_terminal_input(timeout: Duration) -> io::Result<bool> { #[cfg(test)] { let _ = timeout; return terminal_test_state().lock().expect("terminal test state mutex should not be poisoned").poll_results.pop_front().unwrap_or(Ok(false)); } #[cfg(not(test))] { event::poll(timeout) } }

#[rustfmt::skip]
fn read_terminal_input() -> io::Result<Event> { #[cfg(test)] { return terminal_test_state().lock().expect("terminal test state mutex should not be poisoned").read_results.pop_front().unwrap_or_else(|| Err(io::Error::other("missing queued terminal test event"))); } #[cfg(not(test))] { event::read() } }

pub(crate) fn run_terminal_ui(runtime_handle: Handle, state: UiRunState) -> Result<()> {
    let (mut terminal, mut setup_guard) = setup_terminal()?;
    let result = run_event_loop(&mut terminal, runtime_handle, state);
    let cleanup_result = restore_terminal(&mut terminal, &mut setup_guard);
    cleanup_result?;
    result
}

fn setup_terminal() -> Result<(Terminal<CrosstermBackend<io::Stdout>>, TerminalSetupGuard)> {
    let mut guard = TerminalSetupGuard::default();
    enable_terminal_raw_mode().context(SetupTerminalUiSnafu)?;
    guard.raw_mode_enabled = true;

    let mut stdout = io::stdout();
    enter_terminal_alternate_screen(&mut stdout).context(SetupTerminalUiSnafu)?;
    guard.alternate_screen_enabled = true;

    let terminal = build_terminal(stdout).context(SetupTerminalUiSnafu)?;
    Ok((terminal, guard))
}

#[rustfmt::skip]
#[allow(clippy::possible_missing_else)]
fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, guard: &mut TerminalSetupGuard) -> Result<()> { if guard.raw_mode_enabled { disable_terminal_raw_mode().context(SetupTerminalUiSnafu)?; guard.raw_mode_enabled = false; } if guard.alternate_screen_enabled { leave_terminal_alternate_screen_backend(terminal.backend_mut()).context(SetupTerminalUiSnafu)?; guard.alternate_screen_enabled = false; } show_terminal_cursor(terminal).context(SetupTerminalUiSnafu) }

pub(super) fn read_terminal_event() -> Result<Option<Event>> {
    if !poll_terminal_input(UI_POLL_INTERVAL).context(PollTerminalInputSnafu)? {
        return Ok(None);
    }
    read_terminal_input()
        .context(ReadTerminalInputSnafu)
        .map(Some)
}

#[cfg(test)]
mod tests {
    use reqwest::Client;
    use tokio::sync::mpsc;

    use super::*;
    use crate::tui::runtime::UiEventChannel;
    use crate::{
        ChatSession,
        contract_sessions::{SessionSnapshot, SessionStatus},
    };

    fn reset_terminal_test_hooks() {
        *terminal_test_state()
            .lock()
            .expect("terminal test state mutex should not be poisoned") =
            TerminalTestState::default();
        *terminal_event_loop_override()
            .lock()
            .expect("terminal event loop override mutex should not be poisoned") = None;
    }

    fn build_state() -> UiRunState {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        UiRunState::new(
            Client::builder().build().expect("client should build"),
            "http://127.0.0.1:8080".to_string(),
            "developer".to_string(),
            ChatSession {
                session: SessionSnapshot {
                    id: "s_test".to_string(),
                    workspace_id: "w_test".to_string(),
                    title: "New chat".to_string(),
                    status: SessionStatus::Active,
                    latest_sequence: 0,
                    messages: Vec::new(),
                    pending_permissions: Vec::new(),
                },
                resume_history: Vec::new(),
                resumed: false,
            },
            Vec::new(),
            Vec::new(),
            UiEventChannel {
                tx: event_tx,
                rx: event_rx,
            },
        )
    }

    fn successful_event_loop(
        _terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
        _runtime_handle: Handle,
        _state: UiRunState,
    ) -> Result<()> {
        Ok(())
    }

    #[test]
    fn terminal_setup_guard_drop_cleans_up_enabled_modes() {
        reset_terminal_test_hooks();

        drop(TerminalSetupGuard {
            raw_mode_enabled: true,
            alternate_screen_enabled: true,
        });

        let state = terminal_test_state()
            .lock()
            .expect("terminal test state mutex should not be poisoned");
        assert_eq!(state.disable_raw_mode_calls, 1);
        assert_eq!(state.leave_alternate_screen_calls, 1);
    }

    #[test]
    fn setup_and_restore_terminal_update_guard_state() {
        reset_terminal_test_hooks();

        let (mut terminal, mut guard) = setup_terminal().expect("test terminal should set up");
        assert!(guard.raw_mode_enabled);
        assert!(guard.alternate_screen_enabled);

        restore_terminal(&mut terminal, &mut guard).expect("test terminal should restore");
        assert!(!guard.raw_mode_enabled);
        assert!(!guard.alternate_screen_enabled);

        let state = terminal_test_state()
            .lock()
            .expect("terminal test state mutex should not be poisoned");
        assert_eq!(state.enable_raw_mode_calls, 1);
        assert_eq!(state.enter_alternate_screen_calls, 1);
        assert_eq!(state.disable_raw_mode_calls, 1);
        assert_eq!(state.leave_alternate_screen_calls, 1);
        assert_eq!(state.show_cursor_calls, 1);
    }

    #[test]
    fn read_terminal_event_returns_none_when_no_event_is_ready() {
        reset_terminal_test_hooks();
        terminal_test_state()
            .lock()
            .expect("terminal test state mutex should not be poisoned")
            .poll_results
            .push_back(Ok(false));

        assert!(
            read_terminal_event()
                .expect("polling should succeed")
                .is_none()
        );
    }

    #[test]
    fn read_terminal_event_returns_the_polled_event() {
        reset_terminal_test_hooks();
        let mut state = terminal_test_state()
            .lock()
            .expect("terminal test state mutex should not be poisoned");
        state.poll_results.push_back(Ok(true));
        state
            .read_results
            .push_back(Ok(Event::Paste("/quit\r".to_string())));
        drop(state);

        assert!(matches!(
            read_terminal_event().expect("reading the queued event should succeed"),
            Some(Event::Paste(data)) if data == "/quit\r"
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_terminal_ui_uses_the_event_loop_override_and_restores_the_terminal() {
        reset_terminal_test_hooks();
        *terminal_event_loop_override()
            .lock()
            .expect("terminal event loop override mutex should not be poisoned") =
            Some(successful_event_loop);

        run_terminal_ui(Handle::current(), build_state())
            .expect("terminal UI should complete through the override");

        let state = terminal_test_state()
            .lock()
            .expect("terminal test state mutex should not be poisoned");
        assert_eq!(state.enable_raw_mode_calls, 1);
        assert_eq!(state.enter_alternate_screen_calls, 1);
        assert_eq!(state.disable_raw_mode_calls, 1);
        assert_eq!(state.leave_alternate_screen_calls, 1);
        assert_eq!(state.show_cursor_calls, 1);
    }
}
