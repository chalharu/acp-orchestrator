use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
};
use unicode_width::UnicodeWidthStr;

use super::app::ChatApp;

struct PaneLayout {
    session: Rect,
    transcript: Rect,
    tool_status: Rect,
    composer: Rect,
    completion: Option<Rect>,
}

pub(super) fn render(frame: &mut Frame<'_>, app: &ChatApp) {
    let layout = pane_layout(frame.area(), app.completion_menu().is_some());

    render_session_pane(frame, layout.session, app);
    render_transcript_pane(frame, layout.transcript, app);
    render_tool_status_pane(frame, layout.tool_status, app);
    render_composer(frame, layout.composer, app);
    if let Some(completion_area) = layout.completion {
        render_completion_menu(frame, completion_area, app);
    }
}

pub(super) fn transcript_viewport(area: Rect, completion_open: bool) -> Rect {
    let transcript = pane_layout(area, completion_open).transcript;
    Rect::new(
        transcript.x.saturating_add(1),
        transcript.y.saturating_add(1),
        transcript.width.saturating_sub(2),
        transcript.height.saturating_sub(2),
    )
}

fn pane_layout(area: Rect, completion_open: bool) -> PaneLayout {
    let mut vertical_constraints = vec![Constraint::Min(8), Constraint::Length(3)];
    if completion_open {
        vertical_constraints.insert(1, Constraint::Length(7));
    }
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vertical_constraints)
        .split(area);

    let body = vertical[0];
    let completion = completion_open.then_some(vertical[1]);
    let composer = *vertical.last().expect("layout must include composer");
    let body_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(30),
            Constraint::Min(30),
            Constraint::Length(34),
        ])
        .split(body);

    PaneLayout {
        session: body_chunks[0],
        transcript: body_chunks[1],
        tool_status: body_chunks[2],
        composer,
        completion,
    }
}

fn render_session_pane(frame: &mut Frame<'_>, area: Rect, app: &ChatApp) {
    let mut lines = vec![
        Line::from(format!("session: {}", app.session_id())),
        Line::from(format!("backend: {}", app.server_url())),
        Line::from(format!("connection: {}", app.connection().label())),
    ];
    if let Some(detail) = app.connection().detail() {
        lines.push(Line::from(format!("detail: {detail}")));
    }
    lines.extend([
        Line::from(""),
        Line::styled("keys", Style::default().add_modifier(Modifier::BOLD)),
        Line::from("tab: slash completion"),
        Line::from("enter: submit/apply"),
        Line::from("up/down: history recall"),
        Line::from("pgup/pgdn: transcript scroll"),
        Line::from("end: follow latest"),
        Line::from(""),
        Line::styled("commands", Style::default().add_modifier(Modifier::BOLD)),
    ]);
    if app.command_catalog().is_empty() {
        lines.push(Line::from("/help unavailable"));
    } else {
        lines.extend(app.command_catalog().iter().map(|candidate| {
            Line::from(vec![
                Span::styled(
                    candidate.label.clone(),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!("  {}", candidate.detail)),
            ])
        }));
    }

    let paragraph = Paragraph::new(Text::from(lines))
        .block(
            Block::default()
                .title("Session / Commands")
                .borders(Borders::ALL),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

fn render_transcript_pane(frame: &mut Frame<'_>, area: Rect, app: &ChatApp) {
    let viewport = transcript_viewport(frame.area(), app.completion_menu().is_some());
    let transcript_lines = if app.transcript().is_empty() {
        vec![Line::from("No conversation messages yet.")]
    } else {
        app.transcript()
            .iter()
            .cloned()
            .map(Line::from)
            .collect::<Vec<_>>()
    };
    let mode = if app.follow_transcript() {
        "follow"
    } else {
        "manual"
    };
    let paragraph = Paragraph::new(Text::from(transcript_lines))
        .block(
            Block::default()
                .title(format!("Transcript ({mode})"))
                .borders(Borders::ALL),
        )
        .scroll((
            app.transcript_start(viewport.height as usize, viewport.width as usize) as u16,
            0,
        ))
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

fn render_tool_status_pane(frame: &mut Frame<'_>, area: Rect, app: &ChatApp) {
    let block = Block::default()
        .title("Tool / Status")
        .borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let (permission_lines, status_lines) = tool_status_lines(app);
    let (permission_height, status_height) = tool_status_section_heights(inner, &permission_lines);
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(permission_height),
            Constraint::Min(status_height),
        ])
        .split(inner);

    if permission_height > 0 {
        render_tail_section(frame, sections[0], "pending permissions", &permission_lines);
    }
    if status_height > 0 {
        render_tail_section(frame, sections[1], "recent status", &status_lines);
    }
}

fn tool_status_lines(app: &ChatApp) -> (Vec<String>, Vec<String>) {
    let permission_lines = if app.pending_permissions().is_empty() {
        vec!["none".to_string()]
    } else {
        app.pending_permissions()
            .iter()
            .map(|request| format!("{} {}", request.request_id, request.summary))
            .collect()
    };
    let status_lines = if app.status_entries().is_empty() {
        vec!["none".to_string()]
    } else {
        app.status_entries()
            .iter()
            .map(|message| format!("[status] {message}"))
            .collect()
    };

    (permission_lines, status_lines)
}

fn tool_status_section_heights(inner: Rect, permission_lines: &[String]) -> (u16, u16) {
    let status_min_height = inner.height.min(4);
    let permission_height = inner
        .height
        .saturating_sub(status_min_height)
        .min(section_row_count(
            "pending permissions",
            permission_lines,
            inner.width as usize,
        ) as u16);
    let status_height = inner.height.saturating_sub(permission_height);
    (permission_height, status_height)
}

fn render_composer(frame: &mut Frame<'_>, area: Rect, app: &ChatApp) {
    let paragraph =
        Paragraph::new(app.input()).block(Block::default().title("Composer").borders(Borders::ALL));
    frame.render_widget(paragraph, area);

    let (x, y) = composer_cursor_position(area, app);
    frame.set_cursor_position((x, y));
}

pub(super) fn composer_cursor_position(area: Rect, app: &ChatApp) -> (u16, u16) {
    let max_offset = area.width.saturating_sub(2) as usize;
    let cursor_offset = app.cursor_display_width().min(max_offset) as u16;
    let x = area.x.saturating_add(1).saturating_add(cursor_offset);
    let y = area.y.saturating_add(1);
    (x, y)
}

fn render_completion_menu(frame: &mut Frame<'_>, area: Rect, app: &ChatApp) {
    let menu = app
        .completion_menu()
        .expect("completion menu should be visible before rendering");
    let items = menu
        .candidates()
        .iter()
        .enumerate()
        .map(|(index, candidate)| {
            let line = format!("{}\t{}", candidate.label, candidate.detail);
            let style = if index == menu.selected() {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            ListItem::new(line).style(style)
        })
        .collect::<Vec<_>>();
    let list = List::new(items).block(
        Block::default()
            .title("Slash Completion")
            .borders(Borders::ALL),
    );
    frame.render_widget(Clear, area);
    frame.render_widget(list, area);
}

fn render_tail_section(frame: &mut Frame<'_>, area: Rect, title: &str, body: &[String]) {
    let lines = section_lines(title, body);
    let paragraph = Paragraph::new(lines.iter().cloned().map(Line::from).collect::<Vec<_>>())
        .scroll((
            tail_scroll(&lines, area.width as usize, area.height as usize),
            0,
        ))
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

fn section_lines(title: &str, body: &[String]) -> Vec<String> {
    std::iter::once(title.to_string())
        .chain(body.iter().cloned())
        .collect()
}

fn section_row_count(title: &str, body: &[String], width: usize) -> usize {
    let lines = section_lines(title, body);
    wrapped_rows(&lines, width)
}

fn tail_scroll(lines: &[String], width: usize, height: usize) -> u16 {
    wrapped_rows(lines, width)
        .saturating_sub(height.max(1))
        .min(u16::MAX as usize) as u16
}

fn wrapped_rows(lines: &[String], width: usize) -> usize {
    lines
        .iter()
        .map(|line| {
            UnicodeWidthStr::width(line.as_str())
                .max(1)
                .div_ceil(width.max(1))
        })
        .sum()
}
