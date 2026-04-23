use crate::contract_messages::{ConversationMessage, MessageRole};
use crate::contract_permissions::PermissionRequest;
use crate::contract_slash::CompletionCandidate;
use unicode_width::UnicodeWidthStr;

use crate::events::StreamUpdate;

const MAX_STATUS_ENTRIES: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ConnectionState {
    Connected,
    Disconnected(String),
    Closed(String),
}

impl ConnectionState {
    pub(super) fn label(&self) -> &str {
        match self {
            ConnectionState::Connected => "connected",
            ConnectionState::Disconnected(_) => "disconnected",
            ConnectionState::Closed(_) => "closed",
        }
    }

    pub(super) fn detail(&self) -> Option<&str> {
        match self {
            ConnectionState::Connected => None,
            ConnectionState::Disconnected(reason) | ConnectionState::Closed(reason) => {
                Some(reason.as_str())
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CompletionMenu {
    candidates: Vec<CompletionCandidate>,
    selected: usize,
}

impl CompletionMenu {
    fn new(candidates: Vec<CompletionCandidate>) -> Self {
        Self {
            candidates,
            selected: 0,
        }
    }

    pub(super) fn candidates(&self) -> &[CompletionCandidate] {
        &self.candidates
    }

    pub(super) fn selected(&self) -> usize {
        self.selected
    }

    pub(super) fn select_next(&mut self) {
        self.selected = (self.selected + 1) % self.candidates.len();
    }

    pub(super) fn select_previous(&mut self) {
        self.selected = self
            .selected
            .checked_sub(1)
            .unwrap_or(self.candidates.len().saturating_sub(1));
    }

    fn current(&self) -> Option<&CompletionCandidate> {
        self.candidates.get(self.selected)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ChatApp {
    session_id: String,
    server_url: String,
    connection: ConnectionState,
    transcript: Vec<String>,
    pending_permissions: Vec<PermissionRequest>,
    status_entries: Vec<String>,
    command_catalog: Vec<CompletionCandidate>,
    input_history: Vec<String>,
    input: String,
    cursor: usize,
    completion_menu: Option<CompletionMenu>,
    history_index: Option<usize>,
    history_draft: Option<String>,
    follow_transcript: bool,
    transcript_scroll: usize,
    should_quit: bool,
}

impl ChatApp {
    pub(super) fn new(
        session_id: &str,
        server_url: &str,
        resumed: bool,
        messages: &[ConversationMessage],
        pending_permissions: &[PermissionRequest],
        command_catalog: Vec<CompletionCandidate>,
    ) -> Self {
        let mut app = Self {
            session_id: session_id.to_string(),
            server_url: server_url.to_string(),
            connection: ConnectionState::Connected,
            transcript: Vec::new(),
            pending_permissions: pending_permissions.to_vec(),
            status_entries: Vec::new(),
            command_catalog,
            input_history: Vec::new(),
            input: String::new(),
            cursor: 0,
            completion_menu: None,
            history_index: None,
            history_draft: None,
            follow_transcript: true,
            transcript_scroll: 0,
            should_quit: false,
        };

        for message in messages {
            app.append_message(message);
        }
        app.push_status(if resumed {
            "resumed existing session"
        } else {
            "new session ready"
        });
        if !app.pending_permissions.is_empty() {
            app.push_status(format!(
                "{} pending permission request(s) need attention",
                app.pending_permissions.len()
            ));
        }

        app
    }

    pub(super) fn session_id(&self) -> &str {
        &self.session_id
    }

    pub(super) fn server_url(&self) -> &str {
        &self.server_url
    }

    pub(super) fn connection(&self) -> &ConnectionState {
        &self.connection
    }

    pub(super) fn transcript(&self) -> &[String] {
        &self.transcript
    }

    pub(super) fn pending_permissions(&self) -> &[PermissionRequest] {
        &self.pending_permissions
    }

    pub(super) fn status_entries(&self) -> &[String] {
        &self.status_entries
    }

    pub(super) fn command_catalog(&self) -> &[CompletionCandidate] {
        &self.command_catalog
    }

    pub(super) fn input(&self) -> &str {
        &self.input
    }

    pub(super) fn cursor(&self) -> usize {
        self.cursor
    }

    pub(super) fn cursor_display_width(&self) -> usize {
        UnicodeWidthStr::width(&self.input[..self.cursor])
    }

    pub(super) fn completion_menu(&self) -> Option<&CompletionMenu> {
        self.completion_menu.as_ref()
    }

    pub(super) fn should_quit(&self) -> bool {
        self.should_quit
    }

    pub(super) fn follow_transcript(&self) -> bool {
        self.follow_transcript
    }

    pub(super) fn transcript_start(&self, viewport_height: usize, viewport_width: usize) -> usize {
        let max_start = self.max_transcript_start(viewport_height, viewport_width);
        if self.follow_transcript {
            max_start
        } else {
            self.transcript_scroll.min(max_start)
        }
    }

    pub(super) fn insert_char(&mut self, value: char) {
        self.reset_history_navigation();
        self.input.insert(self.cursor, value);
        self.cursor += value.len_utf8();
        self.clear_completion_menu();
    }

    pub(super) fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let previous_index = self.input[..self.cursor]
            .char_indices()
            .last()
            .map(|(index, _)| index)
            .unwrap_or(0);
        self.reset_history_navigation();
        self.input.drain(previous_index..self.cursor);
        self.cursor = previous_index;
        self.clear_completion_menu();
    }

    pub(super) fn move_cursor_left(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.cursor = self.input[..self.cursor]
            .char_indices()
            .last()
            .map(|(index, _)| index)
            .unwrap_or(0);
    }

    pub(super) fn move_cursor_right(&mut self) {
        if self.cursor >= self.input.len() {
            return;
        }
        let mut chars = self.input[self.cursor..].char_indices();
        let next = chars
            .nth(1)
            .map(|(index, _)| self.cursor + index)
            .unwrap_or(self.input.len());
        self.cursor = next;
    }

    pub(super) fn clear_input(&mut self) {
        self.reset_history_navigation();
        self.replace_input(String::new());
    }

    pub(super) fn clear_completion_menu(&mut self) {
        self.completion_menu = None;
    }

    pub(super) fn show_completion_menu(&mut self, candidates: Vec<CompletionCandidate>) {
        self.completion_menu = (!candidates.is_empty()).then(|| CompletionMenu::new(candidates));
    }

    pub(super) fn select_next_completion(&mut self) {
        if let Some(menu) = &mut self.completion_menu {
            menu.select_next();
        }
    }

    pub(super) fn select_previous_completion(&mut self) {
        if let Some(menu) = &mut self.completion_menu {
            menu.select_previous();
        }
    }

    pub(super) fn apply_selected_completion(&mut self) {
        let Some(insert_text) = self
            .completion_menu
            .as_ref()
            .and_then(CompletionMenu::current)
            .map(|candidate| candidate.insert_text.clone())
        else {
            return;
        };

        self.reset_history_navigation();
        let start = completion_start(&self.input[..self.cursor]);
        self.input.replace_range(start..self.cursor, &insert_text);
        self.cursor = start + insert_text.len();
        self.clear_completion_menu();
    }

    pub(super) fn record_submitted_input(&mut self, value: &str) {
        let value = value.trim();
        if value.is_empty() {
            return;
        }
        if self
            .input_history
            .last()
            .is_none_or(|existing| existing != value)
        {
            self.input_history.push(value.to_string());
        }
        self.reset_history_navigation();
    }

    pub(super) fn recall_previous_input(&mut self) {
        if self.input_history.is_empty() {
            return;
        }

        self.history_index = Some(match self.history_index {
            Some(index) => index.saturating_sub(1),
            None => {
                self.history_draft = Some(self.input.clone());
                self.input_history.len().saturating_sub(1)
            }
        });
        if let Some(index) = self.history_index {
            self.replace_input(self.input_history[index].clone());
        }
    }

    pub(super) fn recall_next_input(&mut self) {
        let Some(index) = self.history_index else {
            return;
        };
        if index + 1 < self.input_history.len() {
            let next = index + 1;
            self.history_index = Some(next);
            self.replace_input(self.input_history[next].clone());
            return;
        }

        self.history_index = None;
        let draft = self.history_draft.take().unwrap_or_default();
        self.replace_input(draft);
    }

    pub(super) fn request_quit(&mut self) {
        self.should_quit = true;
    }

    pub(super) fn resume_follow(&mut self) {
        self.follow_transcript = true;
    }

    pub(super) fn scroll_up(
        &mut self,
        viewport_height: usize,
        viewport_width: usize,
        amount: usize,
    ) {
        let max_start = self.max_transcript_start(viewport_height, viewport_width);
        if max_start == 0 {
            self.follow_transcript = true;
            self.transcript_scroll = 0;
            return;
        }
        if self.follow_transcript {
            self.follow_transcript = false;
            self.transcript_scroll = max_start;
        }
        self.transcript_scroll = self.transcript_scroll.saturating_sub(amount);
    }

    pub(super) fn scroll_down(
        &mut self,
        viewport_height: usize,
        viewport_width: usize,
        amount: usize,
    ) {
        let max_start = self.max_transcript_start(viewport_height, viewport_width);
        if max_start == 0 {
            self.follow_transcript = true;
            self.transcript_scroll = 0;
            return;
        }
        if self.follow_transcript {
            return;
        }
        self.transcript_scroll = self.transcript_scroll.saturating_add(amount).min(max_start);
        if self.transcript_scroll >= max_start {
            self.follow_transcript = true;
        }
    }

    pub(super) fn apply_stream_update(&mut self, update: StreamUpdate) {
        match update {
            StreamUpdate::ConversationMessage(message) => self.append_message(&message),
            StreamUpdate::PermissionRequested(request) => {
                if self
                    .pending_permissions
                    .iter()
                    .all(|pending| pending.request_id != request.request_id)
                {
                    self.pending_permissions.push(request);
                }
            }
            StreamUpdate::SessionClosed { reason, .. } => {
                self.connection = ConnectionState::Closed(reason.clone());
                self.push_status(format!("session closed: {reason}"));
            }
            StreamUpdate::Status(message) => self.push_status(message),
        }
    }

    pub(super) fn set_connection_lost(&mut self, message: impl Into<String>) {
        let message = message.into();
        self.connection = ConnectionState::Disconnected(message.clone());
        self.push_status(message);
    }

    pub(super) fn push_status(&mut self, message: impl Into<String>) {
        self.status_entries.push(message.into());
        if self.status_entries.len() > MAX_STATUS_ENTRIES {
            let overflow = self.status_entries.len() - MAX_STATUS_ENTRIES;
            self.status_entries.drain(0..overflow);
        }
    }

    pub(super) fn set_command_catalog(&mut self, command_catalog: Vec<CompletionCandidate>) {
        self.command_catalog = command_catalog;
    }

    pub(super) fn replace_pending_permissions(
        &mut self,
        pending_permissions: Vec<PermissionRequest>,
    ) {
        self.pending_permissions = pending_permissions;
    }

    pub(super) fn remove_pending_permission(&mut self, request_id: &str) {
        self.pending_permissions
            .retain(|request| request.request_id != request_id);
    }

    #[cfg(test)]
    pub(super) fn clear_status_entries(&mut self) {
        self.status_entries.clear();
    }

    fn append_message(&mut self, message: &ConversationMessage) {
        self.transcript
            .extend(formatted_message_lines(message.role.clone(), &message.text));
    }

    fn replace_input(&mut self, value: String) {
        self.input = value;
        self.cursor = self.input.len();
        self.clear_completion_menu();
    }

    fn reset_history_navigation(&mut self) {
        self.history_index = None;
        self.history_draft = None;
    }

    fn max_transcript_start(&self, viewport_height: usize, viewport_width: usize) -> usize {
        self.transcript_row_count(viewport_width)
            .saturating_sub(viewport_height.max(1))
    }

    fn transcript_row_count(&self, viewport_width: usize) -> usize {
        self.transcript
            .iter()
            .map(|line| wrapped_line_rows(line, viewport_width))
            .sum()
    }
}

fn formatted_message_lines(role: MessageRole, text: &str) -> Vec<String> {
    let prefix = match role {
        MessageRole::User => "[user]",
        MessageRole::Assistant => "[assistant]",
    };
    let mut lines = text.lines();
    let first = lines.next().unwrap_or_default();
    std::iter::once(format!("{prefix} {first}"))
        .chain(lines.map(|line| format!("  {line}")))
        .collect()
}

fn completion_start(prefix: &str) -> usize {
    prefix
        .rsplit_once(' ')
        .map_or(0, |(before, _)| before.len() + 1)
}

fn wrapped_line_rows(line: &str, viewport_width: usize) -> usize {
    let viewport_width = viewport_width.max(1);
    UnicodeWidthStr::width(line).max(1).div_ceil(viewport_width)
}
