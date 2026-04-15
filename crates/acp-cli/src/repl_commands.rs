use crate::{
    Result,
    api::{cancel_turn, get_slash_completions, resolve_permission},
    events::permission_decision_label,
};
use acp_contracts::{CompletionKind, PermissionDecision, SlashCommand, parse_slash_command};
use reqwest::Client;

pub(super) async fn handle_repl_command(
    command: &str,
    client: &Client,
    base_url: &str,
    auth_token: &str,
    session_id: &str,
) -> Result<bool> {
    let mut parts = command.split_whitespace();
    let name = parts.next().unwrap_or_default();
    let Some(command) = parse_slash_command(name) else {
        println!("[status] unknown command. Use `/help`.");
        return Ok(false);
    };

    match command {
        SlashCommand::Help => {
            print_help(client, base_url, auth_token, session_id).await;
            Ok(false)
        }
        SlashCommand::Quit => Ok(true),
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

async fn print_help(client: &Client, base_url: &str, auth_token: &str, session_id: &str) {
    let response = match get_slash_completions(client, base_url, auth_token, session_id, "/").await
    {
        Ok(response) => response,
        Err(error) => {
            println!("[status] {error}");
            return;
        }
    };
    let command_candidates = response
        .candidates
        .into_iter()
        .filter(|candidate| candidate.kind == CompletionKind::Command)
        .collect::<Vec<_>>();
    if command_candidates.is_empty() {
        println!("[status] no slash commands available");
        return;
    }

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

async fn handle_cancel_command(
    has_extra_args: bool,
    client: &Client,
    base_url: &str,
    auth_token: &str,
    session_id: &str,
) -> Result<bool> {
    if has_extra_args {
        println!("[status] usage: {}", SlashCommand::Cancel.spec().label);
        return Ok(false);
    }

    match cancel_turn(client, base_url, auth_token, session_id).await {
        Ok(response) if response.cancelled => {
            println!("[status] cancel requested for the running turn");
        }
        Ok(_) => {
            println!("[status] no running turn to cancel");
        }
        Err(error) => println!("[status] {error}"),
    }
    Ok(false)
}

async fn handle_permission_command(
    command: SlashCommand,
    request_id: Option<&str>,
    has_extra_args: bool,
    client: &Client,
    base_url: &str,
    auth_token: &str,
    session_id: &str,
) -> Result<bool> {
    let Some(request_id) = request_id else {
        println!("[status] usage: {}", command.spec().label);
        return Ok(false);
    };
    if has_extra_args {
        println!("[status] usage: {}", command.spec().label);
        return Ok(false);
    }

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
        Ok(response) => println!(
            "[status] permission {} {}",
            response.request_id,
            permission_decision_label(&response.decision)
        ),
        Err(error) => println!("[status] {error}"),
    }
    Ok(false)
}

fn permission_decision(command: SlashCommand) -> PermissionDecision {
    if command == SlashCommand::Approve {
        PermissionDecision::Approve
    } else {
        PermissionDecision::Deny
    }
}
