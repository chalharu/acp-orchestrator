use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompletionKind {
    Command,
    Parameter,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompletionCandidate {
    pub label: String,
    pub insert_text: String,
    pub detail: String,
    pub kind: CompletionKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SlashCompletionsResponse {
    pub candidates: Vec<CompletionCandidate>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlashCommand {
    Help,
    Quit,
    Cancel,
    Approve,
    Deny,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlashCommandSpec {
    pub command: SlashCommand,
    pub name: &'static str,
    pub label: &'static str,
    pub insert_text: &'static str,
    pub detail: &'static str,
}

pub const SLASH_COMMAND_SPECS: &[SlashCommandSpec] = &[
    SlashCommandSpec {
        command: SlashCommand::Help,
        name: "/help",
        label: "/help",
        insert_text: "/help",
        detail: "Show available slash commands",
    },
    SlashCommandSpec {
        command: SlashCommand::Quit,
        name: "/quit",
        label: "/quit",
        insert_text: "/quit",
        detail: "Exit chat",
    },
    SlashCommandSpec {
        command: SlashCommand::Cancel,
        name: "/cancel",
        label: "/cancel",
        insert_text: "/cancel",
        detail: "Cancel the running turn",
    },
    SlashCommandSpec {
        command: SlashCommand::Approve,
        name: "/approve",
        label: "/approve <request-id>",
        insert_text: "/approve ",
        detail: "Approve a pending permission request",
    },
    SlashCommandSpec {
        command: SlashCommand::Deny,
        name: "/deny",
        label: "/deny <request-id>",
        insert_text: "/deny ",
        detail: "Deny a pending permission request",
    },
];

impl SlashCommand {
    pub fn spec(self) -> &'static SlashCommandSpec {
        SLASH_COMMAND_SPECS
            .iter()
            .find(|spec| spec.command == self)
            .expect("every slash command must have a corresponding spec")
    }

    pub fn takes_request_id(self) -> bool {
        matches!(self, Self::Approve | Self::Deny)
    }
}

pub fn parse_slash_command(name: &str) -> Option<SlashCommand> {
    SLASH_COMMAND_SPECS
        .iter()
        .find(|spec| spec.name == name)
        .map(|spec| spec.command)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlashCompletionQuery<'a> {
    Commands {
        prefix: &'a str,
    },
    RequestId {
        command: SlashCommand,
        prefix: &'a str,
    },
}

pub fn classify_slash_completion_prefix(prefix: &str) -> Option<SlashCompletionQuery<'_>> {
    let normalized = prefix.trim_start();
    if normalized.is_empty() || !normalized.starts_with('/') {
        return None;
    }

    if let Some((name, argument_prefix)) = normalized.split_once(' ') {
        let command = parse_slash_command(name)?;
        if !command.takes_request_id() {
            return None;
        }

        let argument_prefix = argument_prefix.trim_start();
        if argument_prefix.chars().any(char::is_whitespace) {
            return None;
        }

        return Some(SlashCompletionQuery::RequestId {
            command,
            prefix: argument_prefix,
        });
    }

    SLASH_COMMAND_SPECS
        .iter()
        .any(|spec| spec.name.starts_with(normalized))
        .then_some(SlashCompletionQuery::Commands { prefix: normalized })
}

#[cfg(test)]
mod tests {
    use super::{
        CompletionCandidate, CompletionKind, SlashCommand, SlashCompletionQuery,
        classify_slash_completion_prefix, parse_slash_command,
    };

    #[test]
    fn completion_candidates_serialize_kind_in_snake_case() {
        let payload = serde_json::to_value(CompletionCandidate {
            label: "/approve <request-id>".to_string(),
            insert_text: "/approve ".to_string(),
            detail: "Approve a pending permission request".to_string(),
            kind: CompletionKind::Command,
        })
        .expect("completion candidates should serialize");

        assert_eq!(payload["kind"], "command");
    }

    #[test]
    fn slash_completion_queries_only_allow_known_command_shapes() {
        assert!(matches!(
            classify_slash_completion_prefix("/ap"),
            Some(SlashCompletionQuery::Commands { prefix: "/ap" })
        ));
        assert!(matches!(
            classify_slash_completion_prefix("  /approve req_"),
            Some(SlashCompletionQuery::RequestId {
                command: SlashCommand::Approve,
                prefix: "req_",
            })
        ));
        assert!(classify_slash_completion_prefix("/approve req_1 extra").is_none());
        assert!(classify_slash_completion_prefix("/home/alice").is_none());
        assert!(classify_slash_completion_prefix("/quit now").is_none());
    }

    #[test]
    fn slash_command_specs_cover_labels_and_request_id_requirements() {
        assert_eq!(SlashCommand::Approve.spec().label, "/approve <request-id>");
        assert_eq!(SlashCommand::Deny.spec().insert_text, "/deny ");
        assert!(SlashCommand::Approve.takes_request_id());
        assert!(!SlashCommand::Help.takes_request_id());
        assert_eq!(parse_slash_command("/cancel"), Some(SlashCommand::Cancel));
    }
}
