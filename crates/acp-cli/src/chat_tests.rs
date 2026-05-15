use super::*;

use crate::contract_errors::ErrorResponse;
use crate::contract_messages::MessageRole;
use crate::contract_sessions::{
    CloseSessionResponse, CreateSessionResponse, SessionHistoryResponse, SessionListItem,
    SessionListResponse, SessionSnapshot, SessionStatus,
};
use crate::contract_workspaces::{WorkspaceListResponse, WorkspaceSummary};
use reqwest::Client;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

#[tokio::test]
async fn run_chat_with_ui_loads_new_sessions_before_launching_the_ui() {
    let server_url = spawn_ordered_http_server(vec![json_response(
        &serde_json::to_vec(&active_session_response("s_new", "w_test"))
            .expect("session response should serialize"),
    )])
    .await;
    let (tx, rx) = tokio::sync::oneshot::channel();

    run_chat_with_ui(
        ChatArgs {
            new: true,
            session_id: None,
            workspace_id: Some("w_test".to_string()),
            server_url: Some(server_url.clone()),
            auth_token: Some("developer".to_string()),
        },
        true,
        move |_client, ui_server_url, _auth_token, chat_session| async move {
            let _ = tx.send((ui_server_url, chat_session.session.id));
            Ok(())
        },
    )
    .await
    .expect("interactive branch should succeed");

    assert_eq!(
        rx.await.expect("ui capture should arrive"),
        (server_url, "s_new".to_string())
    );
}

#[tokio::test]
async fn run_chat_with_handlers_uses_the_noninteractive_repl_path() {
    let server_url = spawn_ordered_http_server(vec![
        json_response(
            &serde_json::to_vec(&active_session_response("s_line", "w_test"))
                .expect("session response should serialize"),
        ),
        sse_response(b"data: {\"sequence\":1,\"kind\":\"status\",\"message\":\"working\"}\n\n"),
    ])
    .await;
    let (tx, rx) = tokio::sync::oneshot::channel();

    run_chat_with_handlers(
        ChatArgs {
            new: true,
            session_id: None,
            workspace_id: Some("w_test".to_string()),
            server_url: Some(server_url.clone()),
            auth_token: Some("developer".to_string()),
        },
        false,
        |_client, _server_url, _auth_token, _chat_session| async { Ok(()) },
        move |_client, repl_server_url, auth_token, session_id| async move {
            let _ = tx.send((repl_server_url, auth_token, session_id));
            Ok(())
        },
    )
    .await
    .expect("non-interactive branch should succeed");

    assert_eq!(
        rx.await.expect("repl capture should arrive"),
        (server_url, "developer".to_string(), "s_line".to_string(),)
    );
}

#[tokio::test]
async fn run_chat_with_handlers_does_not_start_repl_for_closed_sessions() {
    let server_url = spawn_ordered_http_server(vec![
        json_response(
            &serde_json::to_vec(&CreateSessionResponse {
                session: SessionSnapshot {
                    id: "s_resume".to_string(),
                    workspace_id: "w_test".to_string(),
                    title: "New chat".to_string(),
                    status: SessionStatus::Closed,
                    latest_sequence: 2,
                    messages: Vec::new(),
                    pending_permissions: Vec::new(),
                    active_turn: false,
                },
            })
            .expect("session response should serialize"),
        ),
        json_response(
            &serde_json::to_vec(&resumed_history_response())
                .expect("history response should serialize"),
        ),
    ])
    .await;
    let repl_called = Arc::new(AtomicBool::new(false));
    let repl_called_clone = repl_called.clone();

    run_chat_with_handlers(
        resumed_chat_args(&server_url),
        false,
        |_client, _server_url, _auth_token, _chat_session| async { Ok(()) },
        move |_client, _server_url, _auth_token, _session_id| {
            repl_called_clone.store(true, Ordering::SeqCst);
            async move { Ok(()) }
        },
    )
    .await
    .expect("closed sessions should render as read-only transcripts");

    assert!(!repl_called.load(Ordering::SeqCst));
}

#[tokio::test]
async fn load_chat_session_loads_history_for_resumed_sessions() {
    let server_url = spawn_ordered_http_server(vec![
        json_response(
            &serde_json::to_vec(&resumed_session_response())
                .expect("session response should serialize"),
        ),
        json_response(
            &serde_json::to_vec(&resumed_history_response())
                .expect("history response should serialize"),
        ),
    ])
    .await;
    let client = Client::builder().build().expect("client should build");

    let args = resumed_chat_args(&server_url);
    let chat_session = load_chat_session(
        &client,
        &server_url,
        args.new,
        args.session_id.as_deref(),
        args.workspace_id.as_deref(),
        args.auth_token.as_deref().unwrap_or("developer"),
    )
    .await
    .expect("resumed sessions should load");

    assert!(chat_session.resumed);
    assert_eq!(chat_session.resume_history[0].text, "from history");
    assert_eq!(
        chat_session.session.pending_permissions[0].request_id,
        "req_1"
    );
}

#[tokio::test]
async fn load_chat_session_falls_back_to_snapshot_messages_when_history_is_missing() {
    let server_url = spawn_ordered_http_server(vec![
        json_response(
            &serde_json::to_vec(&CreateSessionResponse {
                session: SessionSnapshot {
                    id: "s_resume".to_string(),
                    workspace_id: "w_test".to_string(),
                    title: "New chat".to_string(),
                    status: SessionStatus::Active,
                    latest_sequence: 2,
                    messages: vec![crate::contract_messages::ConversationMessage {
                        id: "m_snapshot".to_string(),
                        role: MessageRole::Assistant,
                        text: "from snapshot".to_string(),
                        created_at: chrono::Utc::now(),
                    }],
                    pending_permissions: Vec::new(),
                    active_turn: false,
                },
            })
            .expect("session response should serialize"),
        ),
        json_error_response("404 Not Found", "session not found"),
    ])
    .await;
    let client = Client::builder().build().expect("client should build");

    let args = resumed_chat_args(&server_url);
    let chat_session = load_chat_session(
        &client,
        &server_url,
        args.new,
        args.session_id.as_deref(),
        args.workspace_id.as_deref(),
        args.auth_token.as_deref().unwrap_or("developer"),
    )
    .await
    .expect("snapshot fallback should succeed");

    assert!(chat_session.resumed);
    assert_eq!(chat_session.resume_history.len(), 1);
    assert_eq!(chat_session.resume_history[0].text, "from snapshot");
}

#[tokio::test]
async fn load_chat_session_returns_non_404_history_errors() {
    let server_url = spawn_ordered_http_server(vec![
        json_response(
            &serde_json::to_vec(&resumed_session_response())
                .expect("session response should serialize"),
        ),
        json_error_response("500 Internal Server Error", "history unavailable"),
    ])
    .await;
    let client = Client::builder().build().expect("client should build");

    let args = resumed_chat_args(&server_url);
    let error = load_chat_session(
        &client,
        &server_url,
        args.new,
        args.session_id.as_deref(),
        args.workspace_id.as_deref(),
        args.auth_token.as_deref().unwrap_or("developer"),
    )
    .await
    .expect_err("unexpected history failures should surface");

    assert!(matches!(
        error,
        CliError::HttpStatus { status, message, .. }
            if status == StatusCode::INTERNAL_SERVER_ERROR && message == "history unavailable"
    ));
}

#[tokio::test]
async fn load_chat_session_surfaces_missing_sessions() {
    let server_url = spawn_ordered_http_server(vec![json_error_response(
        "404 Not Found",
        "session not found",
    )])
    .await;
    let client = Client::builder().build().expect("client should build");

    let args = resumed_chat_args(&server_url);
    let error = load_chat_session(
        &client,
        &server_url,
        args.new,
        args.session_id.as_deref(),
        args.workspace_id.as_deref(),
        args.auth_token.as_deref().unwrap_or("developer"),
    )
    .await
    .expect_err("missing sessions should fail");

    assert!(matches!(
        error,
        CliError::HttpStatus { status, .. } if status == StatusCode::NOT_FOUND
    ));
}

#[tokio::test]
async fn load_chat_session_creates_new_chat_in_explicit_workspace() {
    let server_url = spawn_ordered_http_server(vec![json_response(
        &serde_json::to_vec(&active_session_response("s_workspace", "w_explicit"))
            .expect("session response should serialize"),
    )])
    .await;
    let client = Client::builder().build().expect("client should build");

    let chat_session = load_chat_session(
        &client,
        &server_url,
        true,
        None,
        Some("w_explicit"),
        "developer",
    )
    .await
    .expect("explicit workspace should create a new chat");

    assert_eq!(chat_session.session.workspace_id, "w_explicit");
    assert!(!chat_session.resumed);
}

#[tokio::test]
async fn load_chat_session_requires_workspace_when_auto_selection_is_ambiguous() {
    let server_url = spawn_ordered_http_server(vec![json_response(
        &serde_json::to_vec(&WorkspaceListResponse {
            workspaces: vec![
                workspace_summary("w_one", "One"),
                workspace_summary("w_two", "Two"),
            ],
        })
        .expect("workspace list response should serialize"),
    )])
    .await;
    let client = Client::builder().build().expect("client should build");

    let error = load_chat_session(&client, &server_url, true, None, None, "developer")
        .await
        .expect_err("multiple workspaces should require --workspace");

    assert!(matches!(
        error,
        CliError::WorkspaceSelectionRequired { reason } if reason == "multiple workspaces exist"
    ));
}

#[tokio::test]
async fn load_chat_session_auto_selects_the_only_workspace() {
    let server_url = spawn_ordered_http_server(vec![
        json_response(
            &serde_json::to_vec(&WorkspaceListResponse {
                workspaces: vec![workspace_summary("w_only", "Only")],
            })
            .expect("workspace list response should serialize"),
        ),
        json_response(
            &serde_json::to_vec(&active_session_response("s_auto", "w_only"))
                .expect("session response should serialize"),
        ),
    ])
    .await;
    let client = Client::builder().build().expect("client should build");

    let chat_session = load_chat_session(&client, &server_url, true, None, None, "developer")
        .await
        .expect("single workspace should be auto-selected");

    assert_eq!(chat_session.session.workspace_id, "w_only");
    assert!(!chat_session.resumed);
}

#[tokio::test]
async fn load_chat_session_requires_workspace_when_none_exist() {
    let server_url = spawn_ordered_http_server(vec![json_response(
        &serde_json::to_vec(&WorkspaceListResponse {
            workspaces: Vec::new(),
        })
        .expect("workspace list response should serialize"),
    )])
    .await;
    let client = Client::builder().build().expect("client should build");

    let error = load_chat_session(&client, &server_url, true, None, None, "developer")
        .await
        .expect_err("missing workspace should require --workspace");

    assert!(matches!(
        error,
        CliError::WorkspaceSelectionRequired { reason } if reason == "no workspaces exist"
    ));
}

#[tokio::test]
async fn run_with_args_routes_workspace_list_and_allows_empty_results() {
    let server_url = spawn_ordered_http_server(vec![json_response(
        &serde_json::to_vec(&WorkspaceListResponse {
            workspaces: Vec::new(),
        })
        .expect("workspace list response should serialize"),
    )])
    .await;

    run_with_args([
        "acp",
        "workspace",
        "list",
        "--server-url",
        &server_url,
        "--auth-token",
        "developer",
    ])
    .await
    .expect("empty workspace list should still succeed");
}

#[tokio::test]
async fn run_workspace_list_and_create_cover_workspace_commands() {
    let server_url = spawn_ordered_http_server(vec![
        json_response(
            &serde_json::to_vec(&WorkspaceListResponse {
                workspaces: vec![workspace_summary("w_list", "Listed")],
            })
            .expect("workspace list response should serialize"),
        ),
        json_response(
            &serde_json::to_vec(&crate::contract_workspaces::CreateWorkspaceResponse {
                workspace: acp_contracts_workspaces::WorkspaceDetail {
                    workspace_id: "w_created".to_string(),
                    name: "Created".to_string(),
                    upstream_url: Some("https://example.com/repo.git".to_string()),
                    credential_reference_id: None,
                    bootstrap_kind: None,
                    status: "active".to_string(),
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                },
            })
            .expect("workspace create response should serialize"),
        ),
    ])
    .await;

    run_workspace(WorkspaceArgs {
        command: WorkspaceCommand::List(WorkspaceListArgs {
            server_url: Some(server_url.clone()),
            auth_token: Some("developer".to_string()),
        }),
    })
    .await
    .expect("workspace list should succeed");

    run_workspace(WorkspaceArgs {
        command: WorkspaceCommand::Create(WorkspaceCreateArgs {
            name: "Created".to_string(),
            upstream_url: "https://example.com/repo.git".to_string(),
            server_url: Some(server_url),
            auth_token: Some("developer".to_string()),
        }),
    })
    .await
    .expect("workspace create should succeed");
}

#[tokio::test]
async fn run_session_list_and_close_cover_in_process_session_commands() {
    let server_url = spawn_ordered_http_server(vec![
        json_response(
            &serde_json::to_vec(&session_list_response("s_close"))
                .expect("session list response should serialize"),
        ),
        json_response(
            &serde_json::to_vec(&close_session_response("s_close"))
                .expect("close response should serialize"),
        ),
    ])
    .await;

    run_session(SessionArgs {
        command: SessionCommand::List(ListArgs {
            server_url: Some(server_url.clone()),
            auth_token: Some("developer".to_string()),
        }),
    })
    .await
    .expect("session list should succeed");

    run_session(SessionArgs {
        command: SessionCommand::Close(CloseArgs {
            session_id: "s_close".to_string(),
            server_url: Some(server_url),
            auth_token: Some("developer".to_string()),
        }),
    })
    .await
    .expect("session close should succeed");
}

fn session_list_response(session_id: &str) -> SessionListResponse {
    SessionListResponse {
        sessions: vec![SessionListItem {
            id: session_id.to_string(),
            workspace_id: "w_test".to_string(),
            title: "New chat".to_string(),
            status: SessionStatus::Active,
            last_activity_at: chrono::Utc::now(),
        }],
    }
}

fn close_session_response(session_id: &str) -> CloseSessionResponse {
    CloseSessionResponse {
        session: SessionSnapshot {
            id: session_id.to_string(),
            workspace_id: "w_test".to_string(),
            title: "New chat".to_string(),
            status: SessionStatus::Closed,
            latest_sequence: 3,
            messages: Vec::new(),
            pending_permissions: Vec::new(),
            active_turn: false,
        },
    }
}

async fn spawn_ordered_http_server(responses: Vec<Vec<u8>>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("server should bind");
    let address = listener
        .local_addr()
        .expect("server address should be readable");

    tokio::spawn(async move {
        for response in responses {
            let (mut stream, _) = listener.accept().await.expect("server should accept");
            let mut buffer = [0u8; 4096];
            let _ = stream.read(&mut buffer).await;
            stream
                .write_all(&response)
                .await
                .expect("response should write");
        }
    });

    format!("http://{address}")
}

fn json_response(payload: &[u8]) -> Vec<u8> {
    raw_http_response("200 OK", "application/json", payload)
}

fn json_error_response(status: &str, message: &str) -> Vec<u8> {
    let payload = serde_json::to_vec(&ErrorResponse {
        error: message.to_string(),
    })
    .expect("error payload should serialize");
    raw_http_response(status, "application/json", &payload)
}

fn sse_response(payload: &[u8]) -> Vec<u8> {
    raw_http_response("200 OK", "text/event-stream", payload)
}

fn raw_http_response(status: &str, content_type: &str, payload: &[u8]) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    )
    .into_bytes()
    .into_iter()
    .chain(payload.iter().copied())
    .collect()
}

fn active_session_response(session_id: &str, workspace_id: &str) -> CreateSessionResponse {
    CreateSessionResponse {
        session: SessionSnapshot {
            id: session_id.to_string(),
            workspace_id: workspace_id.to_string(),
            title: "New chat".to_string(),
            status: SessionStatus::Active,
            latest_sequence: 1,
            messages: Vec::new(),
            pending_permissions: Vec::new(),
            active_turn: false,
        },
    }
}

fn resumed_chat_args(server_url: &str) -> ChatArgs {
    ChatArgs {
        new: false,
        session_id: Some("s_resume".to_string()),
        workspace_id: None,
        server_url: Some(server_url.to_string()),
        auth_token: Some("developer".to_string()),
    }
}

fn workspace_summary(workspace_id: &str, name: &str) -> WorkspaceSummary {
    WorkspaceSummary {
        workspace_id: workspace_id.to_string(),
        name: name.to_string(),
        upstream_url: Some(format!("https://example.com/{workspace_id}.git")),
        bootstrap_kind: None,
        status: "active".to_string(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }
}

fn resumed_history_response() -> SessionHistoryResponse {
    SessionHistoryResponse {
        session_id: "s_resume".to_string(),
        messages: vec![crate::contract_messages::ConversationMessage {
            id: "m_1".to_string(),
            role: MessageRole::Assistant,
            text: "from history".to_string(),
            created_at: chrono::Utc::now(),
        }],
    }
}

fn resumed_session_response() -> CreateSessionResponse {
    CreateSessionResponse {
        session: SessionSnapshot {
            id: "s_resume".to_string(),
            workspace_id: "w_test".to_string(),
            title: "New chat".to_string(),
            status: SessionStatus::Active,
            latest_sequence: 2,
            messages: Vec::new(),
            pending_permissions: vec![crate::contract_permissions::PermissionRequest {
                request_id: "req_1".to_string(),
                summary: "read_text_file README.md".to_string(),
                tool_call: None,
            }],
            active_turn: false,
        },
    }
}
