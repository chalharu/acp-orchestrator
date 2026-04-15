use crate::{
    Result,
    api::{cancel_turn, get_slash_completions, resolve_permission},
    events::permission_decision_label,
};
use acp_contracts::{
    CompletionCandidate, CompletionKind, PermissionDecision, SlashCommand, parse_slash_command,
};
use reqwest::Client;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ReplCommandNotice {
    Help(Vec<CompletionCandidate>),
    Status(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PendingPermissionsUpdate {
    None,
    Refresh,
    Remove(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ReplCommandOutcome {
    pub(super) notices: Vec<ReplCommandNotice>,
    pub(super) pending_permissions_update: PendingPermissionsUpdate,
    pub(super) should_quit: bool,
}

impl ReplCommandOutcome {
    fn status(message: impl Into<String>) -> Self {
        Self {
            notices: vec![ReplCommandNotice::Status(message.into())],
            pending_permissions_update: PendingPermissionsUpdate::None,
            should_quit: false,
        }
    }

    fn help(candidates: Vec<CompletionCandidate>) -> Self {
        Self {
            notices: vec![ReplCommandNotice::Help(candidates)],
            pending_permissions_update: PendingPermissionsUpdate::None,
            should_quit: false,
        }
    }

    fn quit() -> Self {
        Self {
            notices: Vec::new(),
            pending_permissions_update: PendingPermissionsUpdate::None,
            should_quit: true,
        }
    }
}

pub(super) async fn handle_repl_command(
    command: &str,
    client: &Client,
    base_url: &str,
    auth_token: &str,
    session_id: &str,
) -> Result<bool> {
    let outcome = execute_repl_command(command, client, base_url, auth_token, session_id).await?;
    print_notices(&outcome.notices);
    Ok(outcome.should_quit)
}

pub(super) async fn execute_repl_command(
    command: &str,
    client: &Client,
    base_url: &str,
    auth_token: &str,
    session_id: &str,
) -> Result<ReplCommandOutcome> {
    let mut parts = command.split_whitespace();
    let name = parts.next().unwrap_or_default();
    let Some(command) = parse_slash_command(name) else {
        return Ok(ReplCommandOutcome::status("unknown command. Use `/help`."));
    };

    match command {
        SlashCommand::Help => load_help_catalog(client, base_url, auth_token, session_id).await,
        SlashCommand::Quit => Ok(ReplCommandOutcome::quit()),
        SlashCommand::Cancel => {
            handle_cancel_command(
                parts.next().is_some(),
                client,
                base_url,
                auth_token,
                session_id,
            )
            .await
        }
        SlashCommand::Approve | SlashCommand::Deny => {
            handle_permission_command(
                command,
                parts.next(),
                parts.next().is_some(),
                client,
                base_url,
                auth_token,
                session_id,
            )
            .await
        }
    }
}

pub(super) async fn load_command_catalog(
    client: &Client,
    base_url: &str,
    auth_token: &str,
    session_id: &str,
) -> Result<Vec<CompletionCandidate>> {
    let response = get_slash_completions(client, base_url, auth_token, session_id, "/").await?;
    Ok(response
        .candidates
        .into_iter()
        .filter(|candidate| candidate.kind == CompletionKind::Command)
        .collect())
}

async fn load_help_catalog(
    client: &Client,
    base_url: &str,
    auth_token: &str,
    session_id: &str,
) -> Result<ReplCommandOutcome> {
    match load_command_catalog(client, base_url, auth_token, session_id).await {
        Ok(command_candidates) if command_candidates.is_empty() => {
            Ok(ReplCommandOutcome::status("no slash commands available"))
        }
        Ok(command_candidates) => Ok(ReplCommandOutcome::help(command_candidates)),
        Err(error) => Ok(ReplCommandOutcome::status(error.to_string())),
    }
}

async fn handle_cancel_command(
    has_extra_args: bool,
    client: &Client,
    base_url: &str,
    auth_token: &str,
    session_id: &str,
) -> Result<ReplCommandOutcome> {
    if has_extra_args {
        return Ok(ReplCommandOutcome::status(format!(
            "usage: {}",
            SlashCommand::Cancel.spec().label
        )));
    }

    Ok(
        match cancel_turn(client, base_url, auth_token, session_id).await {
            Ok(response) if response.cancelled => ReplCommandOutcome {
                notices: vec![ReplCommandNotice::Status(
                    "cancel requested for the running turn".to_string(),
                )],
                pending_permissions_update: PendingPermissionsUpdate::Refresh,
                should_quit: false,
            },
            Ok(_) => ReplCommandOutcome::status("no running turn to cancel"),
            Err(error) => ReplCommandOutcome::status(error.to_string()),
        },
    )
}

async fn handle_permission_command(
    command: SlashCommand,
    request_id: Option<&str>,
    has_extra_args: bool,
    client: &Client,
    base_url: &str,
    auth_token: &str,
    session_id: &str,
) -> Result<ReplCommandOutcome> {
    let Some(request_id) = request_id else {
        return Ok(ReplCommandOutcome::status(format!(
            "usage: {}",
            command.spec().label
        )));
    };
    if has_extra_args {
        return Ok(ReplCommandOutcome::status(format!(
            "usage: {}",
            command.spec().label
        )));
    }

    Ok(
        match resolve_permission(
            client,
            base_url,
            auth_token,
            session_id,
            request_id,
            permission_decision(command),
        )
        .await
        {
            Ok(response) => ReplCommandOutcome {
                notices: vec![ReplCommandNotice::Status(format!(
                    "permission {} {}",
                    response.request_id,
                    permission_decision_label(&response.decision)
                ))],
                pending_permissions_update: PendingPermissionsUpdate::Remove(response.request_id),
                should_quit: false,
            },
            Err(error) => ReplCommandOutcome::status(error.to_string()),
        },
    )
}

fn print_notices(notices: &[ReplCommandNotice]) {
    for notice in notices {
        match notice {
            ReplCommandNotice::Help(candidates) => print_help_catalog(candidates),
            ReplCommandNotice::Status(message) => println!("[status] {message}"),
        }
    }
}

fn print_help_catalog(command_candidates: &[CompletionCandidate]) {
    let label_width = command_candidates
        .iter()
        .map(|candidate| candidate.label.len())
        .max()
        .unwrap_or_default();
    println!("[status] available slash commands:");
    for candidate in command_candidates {
        println!("{:<label_width$}  {}", candidate.label, candidate.detail);
    }
}

fn permission_decision(command: SlashCommand) -> PermissionDecision {
    if command == SlashCommand::Approve {
        PermissionDecision::Approve
    } else {
        PermissionDecision::Deny
    }
}
