use crate::contract_permissions::PermissionDecision;
use crate::contract_slash::{
    CompletionCandidate, CompletionKind, SlashCommand, parse_slash_command,
};
use crate::{
    Result,
    api::{cancel_turn, get_slash_completions, resolve_permission},
    events::permission_decision_label,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract_slash::SlashCompletionsResponse;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    async fn spawn_json_server(body: String) -> String {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("server should bind");
        let address = listener
            .local_addr()
            .expect("server address should be readable");

        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("server should accept");
            let mut buffer = [0u8; 1024];
            let _ = stream.read(&mut buffer).await;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("response should write");
        });

        format!("http://{address}")
    }

    #[tokio::test]
    async fn execute_repl_command_reports_unknown_commands() {
        let client = Client::builder().build().expect("client should build");

        let outcome = execute_repl_command(
            "/unknown",
            &client,
            "http://127.0.0.1",
            "developer",
            "s_test",
        )
        .await
        .expect("unknown commands should not fail");

        assert_eq!(
            outcome,
            ReplCommandOutcome::status("unknown command. Use `/help`.")
        );
    }

    #[tokio::test]
    async fn execute_repl_command_marks_quit_commands() {
        let client = Client::builder().build().expect("client should build");

        let outcome =
            execute_repl_command("/quit", &client, "http://127.0.0.1", "developer", "s_test")
                .await
                .expect("quit commands should not fail");

        assert_eq!(outcome, ReplCommandOutcome::quit());
    }

    #[tokio::test]
    async fn load_command_catalog_filters_non_command_candidates() {
        let payload = serde_json::to_string(&SlashCompletionsResponse {
            candidates: vec![
                CompletionCandidate {
                    label: "/help".to_string(),
                    insert_text: "/help".to_string(),
                    detail: "show help".to_string(),
                    kind: CompletionKind::Command,
                },
                CompletionCandidate {
                    label: "--help".to_string(),
                    insert_text: "--help".to_string(),
                    detail: "parameter".to_string(),
                    kind: CompletionKind::Parameter,
                },
            ],
        })
        .expect("payload should serialize");
        let url = spawn_json_server(payload).await;
        let client = Client::builder().build().expect("client should build");

        let catalog = load_command_catalog(&client, &url, "developer", "s_test")
            .await
            .expect("catalog should load");

        assert_eq!(
            catalog,
            vec![CompletionCandidate {
                label: "/help".to_string(),
                insert_text: "/help".to_string(),
                detail: "show help".to_string(),
                kind: CompletionKind::Command,
            }]
        );
    }

    #[tokio::test]
    async fn execute_repl_command_returns_help_candidates_when_available() {
        let payload = serde_json::to_string(&SlashCompletionsResponse {
            candidates: vec![CompletionCandidate {
                label: "/help".to_string(),
                insert_text: "/help".to_string(),
                detail: "show help".to_string(),
                kind: CompletionKind::Command,
            }],
        })
        .expect("payload should serialize");
        let url = spawn_json_server(payload).await;
        let client = Client::builder().build().expect("client should build");

        let outcome = execute_repl_command("/help", &client, &url, "developer", "s_test")
            .await
            .expect("help commands should not fail");

        assert_eq!(
            outcome,
            ReplCommandOutcome::help(vec![CompletionCandidate {
                label: "/help".to_string(),
                insert_text: "/help".to_string(),
                detail: "show help".to_string(),
                kind: CompletionKind::Command,
            }])
        );
    }

    #[tokio::test]
    async fn execute_repl_command_refreshes_pending_permissions_after_cancel() {
        let payload = serde_json::to_string(&crate::contract_sessions::CancelTurnResponse {
            cancelled: true,
        })
        .expect("payload should serialize");
        let url = spawn_json_server(payload).await;
        let client = Client::builder().build().expect("client should build");

        let outcome = execute_repl_command("/cancel", &client, &url, "developer", "s_test")
            .await
            .expect("cancel commands should not fail");

        assert_eq!(
            outcome,
            ReplCommandOutcome {
                notices: vec![ReplCommandNotice::Status(
                    "cancel requested for the running turn".to_string(),
                )],
                pending_permissions_update: PendingPermissionsUpdate::Refresh,
                should_quit: false,
            }
        );
    }

    #[tokio::test]
    async fn execute_repl_command_removes_resolved_permission_requests() {
        let payload =
            serde_json::to_string(&crate::contract_permissions::ResolvePermissionResponse {
                request_id: "req_1".to_string(),
                decision: PermissionDecision::Deny,
            })
            .expect("payload should serialize");
        let url = spawn_json_server(payload).await;
        let client = Client::builder().build().expect("client should build");

        let outcome = execute_repl_command("/deny req_1", &client, &url, "developer", "s_test")
            .await
            .expect("permission commands should not fail");

        assert_eq!(
            outcome,
            ReplCommandOutcome {
                notices: vec![ReplCommandNotice::Status(
                    "permission req_1 denied".to_string(),
                )],
                pending_permissions_update: PendingPermissionsUpdate::Remove("req_1".to_string()),
                should_quit: false,
            }
        );
    }

    #[test]
    fn permission_decision_matches_slash_command_variants() {
        assert_eq!(
            permission_decision(SlashCommand::Approve),
            PermissionDecision::Approve
        );
        assert_eq!(
            permission_decision(SlashCommand::Deny),
            PermissionDecision::Deny
        );
    }
}
