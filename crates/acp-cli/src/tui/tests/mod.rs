use super::{app::ChatApp, render};
use chrono::Utc;
use ratatui::{Terminal, backend::TestBackend, layout::Rect};

use crate::contract_messages::MessageRole;
use crate::contract_permissions::PermissionRequest;
use crate::contract_slash::{CompletionCandidate, CompletionKind};
use crate::events::StreamUpdate;

mod app;
mod cursor;
mod render_ui;

fn command_candidate(label: &str, detail: &str) -> CompletionCandidate {
    CompletionCandidate {
        label: label.to_string(),
        insert_text: label.to_string(),
        detail: detail.to_string(),
        kind: CompletionKind::Command,
    }
}

fn assistant_message(id: &str, text: &str) -> crate::contract_messages::ConversationMessage {
    crate::contract_messages::ConversationMessage {
        id: id.to_string(),
        role: MessageRole::Assistant,
        text: text.to_string(),
        created_at: Utc::now(),
    }
}

fn user_message(id: &str, text: &str) -> crate::contract_messages::ConversationMessage {
    crate::contract_messages::ConversationMessage {
        id: id.to_string(),
        role: MessageRole::User,
        text: text.to_string(),
        created_at: Utc::now(),
    }
}

fn rendered_screen(app: &ChatApp) -> String {
    let backend = TestBackend::new(120, 30);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    terminal
        .draw(|frame| render::render(frame, app))
        .expect("drawing the slice5 UI should succeed");
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>()
}
