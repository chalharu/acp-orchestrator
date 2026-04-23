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

    lines.join(
        "
",
    )
}

pub(crate) fn format_tool_activity_command(command: &CompletionCandidate) -> String {
    let detail = command.detail.trim();
    if detail.is_empty() {
        format!("- {}", command.label)
    } else {
        format!("- {} — {}", command.label, detail)
    }
}

#[cfg(test)]
mod tests {
    use acp_contracts_slash::{CompletionCandidate, CompletionKind};

    use super::{format_tool_activity_command, tool_activity_text};

    fn command(label: &str, detail: &str) -> CompletionCandidate {
        CompletionCandidate {
            label: label.to_string(),
            insert_text: label.to_string(),
            detail: detail.to_string(),
            kind: CompletionKind::Command,
        }
    }

    #[test]
    fn tool_activity_text_ignores_blank_title_and_detail() {
        assert_eq!(tool_activity_text("  ", "", &[]), "");
        assert_eq!(
            tool_activity_text(" Title ", " detail ", &[]),
            "Title\ndetail"
        );
    }

    #[test]
    fn tool_activity_text_formats_commands_with_and_without_detail() {
        let commands = vec![command("/help", "Show help"), command("/quit", "  ")];

        assert_eq!(
            tool_activity_text("Slash command", "Choose one", &commands),
            "Slash command\nChoose one\nCommands:\n- /help — Show help\n- /quit"
        );
        assert_eq!(
            format_tool_activity_command(&commands[0]),
            "- /help — Show help"
        );
        assert_eq!(format_tool_activity_command(&commands[1]), "- /quit");
    }
}
