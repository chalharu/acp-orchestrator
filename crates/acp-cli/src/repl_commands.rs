use crate::{
    Result,
    api::{cancel_turn, resolve_permission},
    events::permission_decision_label,
};
use acp_contracts::PermissionDecision;
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

    match name {
        "/help" => {
            print_help();
            Ok(false)
        }
        "/quit" => Ok(true),
        "/cancel" => {
            handle_cancel_command(
                parts.next().is_some(),
                client,
                base_url,
                auth_token,
                session_id,
            )
            .await
        }
        "/approve" | "/deny" => {
            handle_permission_command(
                name,
                parts.next(),
                parts.next().is_some(),
                client,
                base_url,
                auth_token,
                session_id,
            )
            .await
        }
        _ => {
            println!("[status] unknown command. Use `/help`.");
            Ok(false)
        }
    }
}

fn print_help() {
    println!("/help");
    println!("/quit");
    println!("/cancel");
    println!("/approve <request-id>");
    println!("/deny <request-id>");
}

async fn handle_cancel_command(
    has_extra_args: bool,
    client: &Client,
    base_url: &str,
    auth_token: &str,
    session_id: &str,
) -> Result<bool> {
    if has_extra_args {
        println!("[status] usage: /cancel");
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
    name: &str,
    request_id: Option<&str>,
    has_extra_args: bool,
    client: &Client,
    base_url: &str,
    auth_token: &str,
    session_id: &str,
) -> Result<bool> {
    let Some(request_id) = request_id else {
        println!("[status] usage: {name} <request-id>");
        return Ok(false);
    };
    if has_extra_args {
        println!("[status] usage: {name} <request-id>");
        return Ok(false);
    }

    match resolve_permission(
        client,
        base_url,
        auth_token,
        session_id,
        request_id,
        permission_decision(name),
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

fn permission_decision(name: &str) -> PermissionDecision {
    if name == "/approve" {
        PermissionDecision::Approve
    } else {
        PermissionDecision::Deny
    }
}
