//! Browser-side slash command logic.
//!
//! All browser slash commands are resolved locally from the shared
//! `SLASH_COMMAND_SPECS` metadata in `acp-contracts`. Only `/help` is
//! supported in the web UI; other commands (cancel, approve, deny, quit)
//! have dedicated on-screen controls.

use acp_contracts::{
    CompletionCandidate, CompletionKind, SLASH_COMMAND_SPECS, SlashCommand, SlashCompletionQuery,
    classify_slash_completion_prefix, parse_slash_command,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum BrowserSlashAction {
    Help,
}

// ---------------------------------------------------------------------------
// Palette visibility & candidate generation
// ---------------------------------------------------------------------------

pub(crate) fn slash_palette_is_visible(draft: &str) -> bool {
    slash_completion_prefix(draft).is_some()
}

pub(crate) fn local_slash_candidates(draft: &str) -> Vec<CompletionCandidate> {
    let Some(SlashCompletionQuery::Commands { prefix }) = classify_slash_completion_prefix(draft)
    else {
        return Vec::new();
    };

    SLASH_COMMAND_SPECS
        .iter()
        .filter(|spec| browser_supports_slash_command(spec.command))
        .filter(|spec| spec.name.starts_with(prefix))
        .map(spec_to_candidate)
        .collect()
}

pub(crate) fn local_browser_commands() -> Vec<CompletionCandidate> {
    SLASH_COMMAND_SPECS
        .iter()
        .filter(|spec| browser_supports_slash_command(spec.command))
        .map(spec_to_candidate)
        .collect()
}

// ---------------------------------------------------------------------------
// Completion application
// ---------------------------------------------------------------------------

pub(crate) fn apply_slash_completion(
    draft: &str,
    candidate: &CompletionCandidate,
) -> Option<String> {
    let normalized = draft.trim_start();
    let leading_whitespace_len = draft.len() - normalized.len();

    match classify_slash_completion_prefix(draft)? {
        SlashCompletionQuery::Commands { .. } => Some(format!(
            "{}{}",
            &draft[..leading_whitespace_len],
            candidate.insert_text
        )),
        SlashCompletionQuery::RequestId { .. } => {
            let argument_start = draft.rfind(' ')? + 1;
            Some(format!(
                "{}{}",
                &draft[..argument_start],
                candidate.insert_text
            ))
        }
    }
}

pub(crate) fn slash_palette_should_apply_on_enter(
    draft: &str,
    candidates: &[CompletionCandidate],
    selected_index: usize,
) -> bool {
    candidates
        .get(selected_index)
        .and_then(|candidate| apply_slash_completion(draft, candidate))
        .is_some_and(|next_draft| next_draft != draft)
}

pub(crate) fn cycle_slash_selection(
    candidate_count: usize,
    current_index: usize,
    forward: bool,
) -> usize {
    if candidate_count == 0 {
        return 0;
    }
    if forward {
        (current_index + 1) % candidate_count
    } else if current_index == 0 {
        candidate_count - 1
    } else {
        current_index - 1
    }
}

// ---------------------------------------------------------------------------
// Browser slash action parsing
// ---------------------------------------------------------------------------

pub(crate) fn parse_browser_slash_action(input: &str) -> Result<BrowserSlashAction, String> {
    let mut parts = input.split_whitespace();
    let name = parts.next().unwrap_or_default();
    let Some(command) = parse_slash_command(name) else {
        return Err("Unknown slash command. Use `/help`.".to_string());
    };
    if !browser_supports_slash_command(command) {
        return Err(unsupported_browser_slash_message(command));
    }

    match command {
        SlashCommand::Help => ensure_no_extra_slash_args(command, parts.next().is_some())
            .map(|()| BrowserSlashAction::Help),
        SlashCommand::Quit | SlashCommand::Cancel | SlashCommand::Approve | SlashCommand::Deny => {
            Err(unsupported_browser_slash_message(command))
        }
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn slash_completion_prefix(draft: &str) -> Option<&str> {
    browser_supports_slash_prefix(draft).then_some(draft)
}

fn browser_supports_slash_prefix(draft: &str) -> bool {
    match classify_slash_completion_prefix(draft) {
        Some(SlashCompletionQuery::Commands { prefix }) => [SlashCommand::Help]
            .into_iter()
            .any(|command| command.spec().name.starts_with(prefix)),
        Some(SlashCompletionQuery::RequestId { command, .. }) => {
            browser_supports_slash_command(command)
        }
        None => false,
    }
}

fn browser_supports_slash_command(command: SlashCommand) -> bool {
    matches!(command, SlashCommand::Help)
}

fn unsupported_browser_slash_message(command: SlashCommand) -> String {
    match command {
        SlashCommand::Help => "Unknown slash command. Use `/help`.".to_string(),
        SlashCommand::Quit => {
            "Use the session list to leave or delete chats in the web UI.".to_string()
        }
        SlashCommand::Cancel | SlashCommand::Approve | SlashCommand::Deny => {
            "Use the on-screen action buttons in the web UI.".to_string()
        }
    }
}

fn ensure_no_extra_slash_args(command: SlashCommand, has_extra_args: bool) -> Result<(), String> {
    if has_extra_args {
        Err(format!("Usage: {}", command.spec().label))
    } else {
        Ok(())
    }
}

fn spec_to_candidate(spec: &acp_contracts::SlashCommandSpec) -> CompletionCandidate {
    CompletionCandidate {
        label: spec.label.to_string(),
        insert_text: spec.insert_text.to_string(),
        detail: spec.detail.to_string(),
        kind: CompletionKind::Command,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acp_contracts::CompletionKind;

    #[test]
    fn slash_palette_visibility_requires_a_supported_prefix() {
        assert!(slash_palette_is_visible("/"));
        assert!(slash_palette_is_visible("/h"));
        assert!(slash_palette_is_visible("/help"));
        assert!(!slash_palette_is_visible(""));
        assert!(!slash_palette_is_visible("hello"));
        assert!(!slash_palette_is_visible("/cancel"));
    }

    #[test]
    fn local_slash_candidates_returns_only_browser_supported_commands() {
        let candidates = local_slash_candidates("/");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].label, "/help");

        let empty = local_slash_candidates("/z");
        assert!(empty.is_empty());
    }

    #[test]
    fn local_browser_commands_returns_all_supported_commands() {
        let commands = local_browser_commands();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].insert_text, "/help");
    }

    #[test]
    fn apply_slash_completion_replaces_command_prefix_and_request_id() {
        let command_candidate = CompletionCandidate {
            label: "/approve <request-id>".to_string(),
            insert_text: "/approve ".to_string(),
            detail: "Approve a pending permission request".to_string(),
            kind: CompletionKind::Command,
        };
        let request_candidate = CompletionCandidate {
            label: "req_1".to_string(),
            insert_text: "req_1".to_string(),
            detail: "read README.md".to_string(),
            kind: CompletionKind::Parameter,
        };

        assert_eq!(
            apply_slash_completion("/ap", &command_candidate).unwrap(),
            "/approve "
        );
        assert_eq!(
            apply_slash_completion("/approve req_", &request_candidate).unwrap(),
            "/approve req_1"
        );
    }

    #[test]
    fn cycle_slash_selection_wraps_in_both_directions() {
        assert_eq!(cycle_slash_selection(3, 0, true), 1);
        assert_eq!(cycle_slash_selection(3, 2, true), 0);
        assert_eq!(cycle_slash_selection(3, 0, false), 2);
        assert_eq!(cycle_slash_selection(0, 0, true), 0);
    }

    #[test]
    fn slash_palette_only_applies_on_enter_when_it_changes_the_draft() {
        let help_candidate = CompletionCandidate {
            label: "/help".to_string(),
            insert_text: "/help".to_string(),
            detail: "Show available slash commands".to_string(),
            kind: CompletionKind::Command,
        };
        let partial_candidate = CompletionCandidate {
            label: "/approve <request-id>".to_string(),
            insert_text: "/approve ".to_string(),
            detail: "Approve a pending permission request".to_string(),
            kind: CompletionKind::Command,
        };

        assert!(!slash_palette_should_apply_on_enter(
            "/help",
            &[help_candidate],
            0
        ));
        assert!(slash_palette_should_apply_on_enter(
            "/ap",
            &[partial_candidate],
            0
        ));
    }

    #[test]
    fn parse_browser_slash_action_handles_help() {
        assert_eq!(
            parse_browser_slash_action("/help").unwrap(),
            BrowserSlashAction::Help
        );
    }

    #[test]
    fn parse_browser_slash_action_rejects_unknown_and_non_web_usage() {
        assert_eq!(
            parse_browser_slash_action("/unknown").unwrap_err(),
            "Unknown slash command. Use `/help`."
        );
        assert_eq!(
            parse_browser_slash_action("/cancel").unwrap_err(),
            "Use the on-screen action buttons in the web UI."
        );
        assert_eq!(
            parse_browser_slash_action("/deny req_1").unwrap_err(),
            "Use the on-screen action buttons in the web UI."
        );
        assert_eq!(
            parse_browser_slash_action("/quit").unwrap_err(),
            "Use the session list to leave or delete chats in the web UI."
        );
    }
}
