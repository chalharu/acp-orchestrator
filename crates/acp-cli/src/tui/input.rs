use acp_contracts::classify_slash_completion_prefix;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;
use reqwest::Client;
use tokio::runtime::Handle;

use super::{SLASH_COMPLETION_TIMEOUT, app::ChatApp, render};
use crate::{
    Result,
    api::{get_session, submit_prompt},
    repl_commands::{
        PendingPermissionsUpdate, ReplCommandNotice, ReplCommandOutcome, execute_repl_command,
    },
};

pub(super) struct UiContext<'a> {
    pub(super) runtime_handle: &'a Handle,
    pub(super) client: &'a Client,
    pub(super) server_url: &'a str,
    pub(super) auth_token: &'a str,
    pub(super) session_id: &'a str,
}

pub(super) fn handle_terminal_event(
    terminal_size: ratatui::layout::Size,
    context: &UiContext<'_>,
    app: &mut ChatApp,
    terminal_event: Event,
) -> Result<()> {
    match terminal_event {
        Event::Key(key) if key.kind != KeyEventKind::Release => {
            let viewport = render::transcript_viewport(
                size_to_rect(terminal_size),
                app.completion_menu().is_some(),
            );
            handle_key(
                context,
                app,
                key,
                viewport.height as usize,
                viewport.width as usize,
            )
        }
        Event::Paste(data) => handle_paste(context, app, &data),
        _ => Ok(()),
    }
}

fn handle_key(
    context: &UiContext<'_>,
    app: &mut ChatApp,
    key: KeyEvent,
    viewport_height: usize,
    viewport_width: usize,
) -> Result<()> {
    if handle_completion_shortcuts(context, app, key)? {
        return Ok(());
    }
    if handle_navigation_key(app, key, viewport_height, viewport_width) {
        return Ok(());
    }
    handle_edit_key(app, key);
    Ok(())
}

fn size_to_rect(size: ratatui::layout::Size) -> Rect {
    Rect::new(0, 0, size.width, size.height)
}

fn handle_completion_shortcuts(
    context: &UiContext<'_>,
    app: &mut ChatApp,
    key: KeyEvent,
) -> Result<bool> {
    match key.code {
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.push_status("interrupted input. Use `/quit` to leave the chat.");
            Ok(true)
        }
        KeyCode::Esc => {
            app.clear_completion_menu();
            Ok(true)
        }
        KeyCode::Tab => {
            if app.completion_menu().is_some() {
                app.select_next_completion();
            } else {
                update_completion_menu(context, app);
            }
            Ok(true)
        }
        KeyCode::BackTab => {
            app.select_previous_completion();
            Ok(true)
        }
        KeyCode::Enter | KeyCode::Char('\n') | KeyCode::Char('\r') => {
            if app.completion_menu().is_some() {
                app.apply_selected_completion();
            } else {
                submit_current_input(context, app)?;
            }
            Ok(true)
        }
        _ => Ok(false),
    }
}

fn handle_navigation_key(
    app: &mut ChatApp,
    key: KeyEvent,
    viewport_height: usize,
    viewport_width: usize,
) -> bool {
    match key.code {
        KeyCode::Up => {
            if app.completion_menu().is_some() {
                app.select_previous_completion();
            } else {
                app.recall_previous_input();
            }
            true
        }
        KeyCode::Down => {
            if app.completion_menu().is_some() {
                app.select_next_completion();
            } else {
                app.recall_next_input();
            }
            true
        }
        KeyCode::PageUp => {
            app.scroll_up(viewport_height, viewport_width, viewport_height.max(1));
            true
        }
        KeyCode::PageDown => {
            app.scroll_down(viewport_height, viewport_width, viewport_height.max(1));
            true
        }
        KeyCode::End => {
            app.resume_follow();
            true
        }
        _ => false,
    }
}

fn handle_edit_key(app: &mut ChatApp, key: KeyEvent) {
    match key.code {
        KeyCode::Backspace => app.backspace(),
        KeyCode::Left => app.move_cursor_left(),
        KeyCode::Right => app.move_cursor_right(),
        KeyCode::Char(value)
            if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
        {
            app.insert_char(value);
        }
        _ => {}
    }
}

fn update_completion_menu(context: &UiContext<'_>, app: &mut ChatApp) {
    let Some(prefix) = completion_query(app.input(), app.cursor()) else {
        app.clear_completion_menu();
        return;
    };

    let response = match context.runtime_handle.block_on(tokio::time::timeout(
        SLASH_COMPLETION_TIMEOUT,
        crate::api::get_slash_completions(
            context.client,
            context.server_url,
            context.auth_token,
            context.session_id,
            prefix,
        ),
    )) {
        Ok(Ok(response)) => response,
        Ok(Err(error)) => {
            app.push_status(format!("slash completion unavailable: {error}"));
            return;
        }
        Err(_) => {
            app.push_status("slash completion timed out");
            return;
        }
    };
    if response.candidates.len() == 1 {
        app.show_completion_menu(response.candidates);
        app.apply_selected_completion();
    } else {
        app.show_completion_menu(response.candidates);
    }
}

fn completion_query(line: &str, cursor: usize) -> Option<&str> {
    let prefix = &line[..cursor];
    classify_slash_completion_prefix(prefix).map(|_| prefix)
}

fn submit_current_input(context: &UiContext<'_>, app: &mut ChatApp) -> Result<()> {
    let line = app.input().trim().to_string();
    if line.is_empty() {
        return Ok(());
    }
    app.record_submitted_input(&line);

    if line.starts_with('/') {
        let command = execute_repl_command(
            &line,
            context.client,
            context.server_url,
            context.auth_token,
            context.session_id,
        );
        let outcome = context.runtime_handle.block_on(command)?;
        app.clear_input();
        apply_command_outcome(context, app, outcome)?;
        return Ok(());
    }

    match context.runtime_handle.block_on(submit_prompt(
        context.client,
        context.server_url,
        context.auth_token,
        context.session_id,
        &line,
    )) {
        Ok(()) => app.clear_input(),
        Err(error) => app.push_status(error.to_string()),
    }
    Ok(())
}

fn handle_paste(context: &UiContext<'_>, app: &mut ChatApp, data: &str) -> Result<()> {
    for value in data.chars() {
        if value == '\n' || value == '\r' {
            submit_current_input(context, app)?;
        } else {
            app.insert_char(value);
        }
    }
    Ok(())
}

fn apply_command_outcome(
    context: &UiContext<'_>,
    app: &mut ChatApp,
    outcome: ReplCommandOutcome,
) -> Result<()> {
    for notice in outcome.notices {
        match notice {
            ReplCommandNotice::Help(candidates) => {
                app.set_command_catalog(candidates);
                app.push_status("available slash commands refreshed");
            }
            ReplCommandNotice::Status(message) => app.push_status(message),
        }
    }
    match outcome.pending_permissions_update {
        PendingPermissionsUpdate::None => {}
        PendingPermissionsUpdate::Refresh => {
            match context.runtime_handle.block_on(get_session(
                context.client,
                context.server_url,
                context.auth_token,
                context.session_id,
            )) {
                Ok(session) => app.replace_pending_permissions(session.pending_permissions),
                Err(error) => app.push_status(error.to_string()),
            }
        }
        PendingPermissionsUpdate::Remove(request_id) => {
            app.remove_pending_permission(&request_id);
        }
    }
    if outcome.should_quit {
        app.request_quit();
    }
    Ok(())
}

#[cfg(test)]
mod tests;
