use agent_client_protocol as acp;
use std::time::Duration;
use tokio::{sync::watch, time::sleep};

pub(super) fn prompt_text(prompt: &[acp::ContentBlock]) -> String {
    prompt
        .iter()
        .map(content_text)
        .collect::<Vec<_>>()
        .join(" ")
}

fn content_text(content: &acp::ContentBlock) -> String {
    match content {
        acp::ContentBlock::Text(text) => text.text.clone(),
        acp::ContentBlock::Image(_) => "<image>".to_string(),
        acp::ContentBlock::Audio(_) => "<audio>".to_string(),
        acp::ContentBlock::ResourceLink(link) => link.uri.clone(),
        content => resource_placeholder(matches!(content, acp::ContentBlock::Resource(_))),
    }
}

fn resource_placeholder(is_resource: bool) -> String {
    ["<unsupported>", "<resource>"][usize::from(is_resource)].to_string()
}

pub(crate) fn reply_for(prompt: &str) -> String {
    let compact = prompt.split_whitespace().collect::<Vec<_>>().join(" ");

    format!(
        "mock assistant: I received `{}`. The backend-to-mock ACP round-trip succeeded.",
        truncate(&compact, 120)
    )
}

pub(super) fn prompt_requires_permission(prompt: &str) -> bool {
    prompt.to_ascii_lowercase().contains("permission")
}

pub(super) async fn wait_for_cancel(
    cancel_rx: &mut watch::Receiver<u64>,
    start_generation: u64,
    response_delay: Duration,
) -> bool {
    if *cancel_rx.borrow() != start_generation {
        return true;
    }

    tokio::select! {
        _ = sleep(response_delay) => false,
        changed = cancel_rx.changed() => changed.is_ok() && *cancel_rx.borrow() != start_generation,
    }
}

fn truncate(value: &str, max_len: usize) -> String {
    let mut chars = value.chars();
    let truncated = chars.by_ref().take(max_len).collect::<String>();

    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}
