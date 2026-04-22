use acp_contracts_slash::CompletionCandidate;

pub(crate) fn tool_activity_text(
    title: &str,
    detail: &str,
    commands: &[CompletionCandidate],
) -> String {
    let mut lines = Vec::new();

    let title = title.trim();
    if !title.is_empty() {
        lines.push(title.to_string());
    }

    let detail = detail.trim();
    if !detail.is_empty() {
        lines.push(detail.to_string());
    }

    if !commands.is_empty() {
        lines.push("Commands:".to_string());
        lines.extend(commands.iter().map(format_tool_activity_command));
    }

    lines.join("
")
}

pub(crate) fn format_tool_activity_command(command: &CompletionCandidate) -> String {
    let detail = command.detail.trim();
    if detail.is_empty() {
        format!("- {}", command.label)
    } else {
        format!("- {} — {}", command.label, detail)
    }
}
