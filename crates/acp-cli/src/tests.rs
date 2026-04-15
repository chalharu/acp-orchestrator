use super::*;
use crate::{
    events::{InitialSnapshotState, render_event, stream_events},
    recent_sessions::{create_recent_sessions_parent, recent_sessions_path_from},
    repl_commands::handle_repl_command,
};
use chrono::TimeZone;
use std::path::Path;
use tokio::{io::AsyncWriteExt, net::TcpListener};

#[tokio::test]
async fn ensure_success_uses_http_reason_when_error_body_is_not_json() {
    let url = spawn_raw_http_server(
        "HTTP/1.1 502 Bad Gateway\r\nContent-Type: text/plain\r\nContent-Length: 11\r\n\r\nbad gateway",
    )
    .await;
    let client = Client::builder().build().expect("client should build");
    let response = client
        .get(&url)
        .send()
        .await
        .expect("request should succeed");

    let error = ensure_success(response, "open event stream")
        .await
        .expect_err("plain text errors should fail");

    assert!(matches!(
        error,
        CliError::HttpStatus { action, status, message }
            if action == "open event stream"
                && status == StatusCode::BAD_GATEWAY
                && message == "Bad Gateway"
    ));
}

#[tokio::test]
async fn handle_repl_command_reports_idle_cancellation_without_failing() {
    let url = spawn_raw_http_server_bytes(
        b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{\"cancelled\":false}\n"
            .to_vec(),
    )
    .await;
    let client = Client::builder().build().expect("client should build");

    let should_quit = handle_repl_command("/cancel", &client, &url, "developer", "s_test")
        .await
        .expect("idle cancellation should succeed");

    assert!(!should_quit);
}

#[tokio::test]
async fn handle_repl_command_validates_cancel_and_permission_usage() {
    let client = Client::builder().build().expect("client should build");

    assert!(
        !handle_repl_command(
            "/cancel extra",
            &client,
            "http://127.0.0.1",
            "developer",
            "s",
        )
        .await
        .expect("usage errors should not fail")
    );
    assert!(
        !handle_repl_command("/approve", &client, "http://127.0.0.1", "developer", "s")
            .await
            .expect("usage errors should not fail")
    );
    assert!(
        !handle_repl_command(
            "/deny req_1 extra",
            &client,
            "http://127.0.0.1",
            "developer",
            "s",
        )
        .await
        .expect("usage errors should not fail")
    );
}

#[tokio::test]
async fn handle_repl_command_reports_cancel_errors_without_failing() {
    let url = spawn_raw_http_server(
        "HTTP/1.1 409 Conflict\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{\"error\":\"cancel failed\"}",
    )
    .await;
    let client = Client::builder().build().expect("client should build");

    let should_quit = handle_repl_command("/cancel", &client, &url, "developer", "s_test")
        .await
        .expect("cancel errors should stay in the REPL");

    assert!(!should_quit);
}

#[tokio::test]
async fn handle_repl_command_reports_permission_errors_without_failing() {
    let url = spawn_raw_http_server(
        "HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{\"error\":\"permission request not found\"}",
    )
    .await;
    let client = Client::builder().build().expect("client should build");

    let should_quit = handle_repl_command("/approve req_1", &client, &url, "developer", "s_test")
        .await
        .expect("permission errors should stay in the REPL");

    assert!(!should_quit);
}

#[tokio::test]
async fn handle_repl_command_reports_help_errors_without_failing() {
    let url = spawn_raw_http_server(
        "HTTP/1.1 500 Internal Server Error\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{\"error\":\"help failed\"}",
    )
    .await;
    let client = Client::builder().build().expect("client should build");

    let should_quit = handle_repl_command("/help", &client, &url, "developer", "s_test")
        .await
        .expect("help failures should stay in the REPL");

    assert!(!should_quit);
}

#[tokio::test]
async fn handle_repl_command_handles_empty_help_catalogs() {
    let url = spawn_raw_http_server(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{\"candidates\":[]}",
    )
    .await;
    let client = Client::builder().build().expect("client should build");

    let should_quit = handle_repl_command("/help", &client, &url, "developer", "s_test")
        .await
        .expect("empty help catalogs should stay in the REPL");

    assert!(!should_quit);
}

#[tokio::test]
async fn stream_events_finishes_when_the_server_closes_the_stream() {
    let url = spawn_raw_http_server(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\ndata: {\"sequence\":1,\"kind\":\"status\",\"message\":\"done\"}\n\n",
    )
    .await;
    let client = Client::builder().build().expect("client should build");

    stream_events(client, url, "developer".to_string(), None)
        .await
        .expect("single-event streams should complete cleanly");
}

#[tokio::test]
async fn stream_events_renders_new_messages_from_an_initial_snapshot_delta() {
    let url = spawn_raw_http_server(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\ndata: {\"sequence\":1,\"kind\":\"session_snapshot\",\"session\":{\"id\":\"s_test\",\"status\":\"active\",\"latest_sequence\":1,\"messages\":[{\"id\":\"m_new\",\"role\":\"assistant\",\"text\":\"hello\",\"created_at\":\"2024-01-01T00:00:00Z\"}]}}\n\n",
    )
    .await;
    let client = Client::builder().build().expect("client should build");

    stream_events(
        client,
        url,
        "developer".to_string(),
        Some(InitialSnapshotState::from_messages_and_permissions(
            &[acp_contracts::ConversationMessage {
                id: "m_known".to_string(),
                role: MessageRole::Assistant,
                text: "already rendered".to_string(),
                created_at: Utc
                    .with_ymd_and_hms(2024, 1, 1, 0, 0, 0)
                    .single()
                    .expect("timestamp should be valid"),
            }],
            &[],
        )),
    )
    .await
    .expect("initial snapshot delta rendering should complete cleanly");
}

#[tokio::test]
async fn stream_events_renders_new_pending_permissions_from_an_initial_snapshot_delta() {
    let url = spawn_raw_http_server(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\ndata: {\"sequence\":1,\"kind\":\"session_snapshot\",\"session\":{\"id\":\"s_test\",\"status\":\"active\",\"latest_sequence\":1,\"messages\":[],\"pending_permissions\":[{\"request_id\":\"req_new\",\"summary\":\"read_text_file README.md\"}]}}\n\n",
    )
    .await;
    let client = Client::builder().build().expect("client should build");

    stream_events(
        client,
        url,
        "developer".to_string(),
        Some(InitialSnapshotState::from_messages_and_permissions(
            &[],
            &[acp_contracts::PermissionRequest {
                request_id: "req_old".to_string(),
                summary: "read_text_file Cargo.toml".to_string(),
            }],
        )),
    )
    .await
    .expect("initial snapshot permission delta should complete cleanly");
}

#[tokio::test]
async fn stream_events_surfaces_event_stream_read_errors() {
    let url = spawn_raw_http_server_bytes(
        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\ndata: \xff\n\n"
            .to_vec(),
    )
    .await;
    let client = Client::builder().build().expect("client should build");

    let error = stream_events(client, url, "developer".to_string(), None)
        .await
        .expect_err("invalid event streams should fail");

    assert!(matches!(error, CliError::ReadEventStream { .. }));
}

#[tokio::test]
async fn stream_events_to_stderr_returns_after_stream_failures() {
    let url = spawn_raw_http_server_bytes(
        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\ndata: \xff\n\n"
            .to_vec(),
    )
    .await;
    let client = Client::builder().build().expect("client should build");

    stream_events_to_stderr(client, url, "developer".to_string(), None).await;
}

#[test]
fn render_event_covers_all_display_variants() {
    let created_at = Utc
        .with_ymd_and_hms(2024, 1, 1, 0, 0, 0)
        .single()
        .expect("timestamp should be valid");
    let snapshot = SessionSnapshot {
        id: "s_test".to_string(),
        status: acp_contracts::SessionStatus::Active,
        latest_sequence: 2,
        messages: vec![acp_contracts::ConversationMessage {
            id: "m_test".to_string(),
            role: MessageRole::Assistant,
            text: "hello".to_string(),
            created_at,
        }],
        pending_permissions: vec![acp_contracts::PermissionRequest {
            request_id: "req_1".to_string(),
            summary: "read_text_file README.md".to_string(),
        }],
    };

    render_event(&StreamEvent {
        sequence: 2,
        payload: StreamEventPayload::SessionSnapshot { session: snapshot },
    });
    render_event(&StreamEvent {
        sequence: 3,
        payload: StreamEventPayload::SessionClosed {
            session_id: "s_test".to_string(),
            reason: "done".to_string(),
        },
    });
    render_event(&StreamEvent {
        sequence: 4,
        payload: StreamEventPayload::PermissionRequested {
            request: acp_contracts::PermissionRequest {
                request_id: "req_1".to_string(),
                summary: "read_text_file README.md".to_string(),
            },
        },
    });
    render_event(&StreamEvent::status(5, "working"));
}

#[test]
fn render_resume_history_uses_loaded_history_messages_and_latest_permissions() {
    let created_at = Utc
        .with_ymd_and_hms(2024, 1, 1, 0, 0, 0)
        .single()
        .expect("timestamp should be valid");
    let chat_session = ChatSession {
        session: SessionSnapshot {
            id: "s_test".to_string(),
            status: acp_contracts::SessionStatus::Active,
            latest_sequence: 2,
            messages: vec![acp_contracts::ConversationMessage {
                id: "m_snapshot".to_string(),
                role: MessageRole::Assistant,
                text: "from snapshot".to_string(),
                created_at,
            }],
            pending_permissions: vec![acp_contracts::PermissionRequest {
                request_id: "req_1".to_string(),
                summary: "read_text_file README.md".to_string(),
            }],
        },
        resume_history: vec![acp_contracts::ConversationMessage {
            id: "m_history".to_string(),
            role: MessageRole::Assistant,
            text: "from history".to_string(),
            created_at,
        }],
        resumed: true,
    };

    assert_eq!(
        render_resume_history(&chat_session),
        Some(InitialSnapshotState::from_messages_and_permissions(
            &chat_session.resume_history,
            &chat_session.session.pending_permissions,
        ))
    );
}

#[test]
fn print_chat_status_handles_pending_permissions() {
    let chat_session = ChatSession {
        session: SessionSnapshot {
            id: "s_test".to_string(),
            status: acp_contracts::SessionStatus::Active,
            latest_sequence: 1,
            messages: Vec::new(),
            pending_permissions: vec![acp_contracts::PermissionRequest {
                request_id: "req_1".to_string(),
                summary: "read_text_file README.md".to_string(),
            }],
        },
        resume_history: Vec::new(),
        resumed: false,
    };

    print_chat_status(&chat_session, true);
}

#[test]
fn recent_sessions_path_uses_the_explicit_path_first() {
    let path = recent_sessions_path_from(
        Some(OsString::from("/tmp/acp-test.json")),
        Some(PathBuf::from("/ignored")),
    )
    .expect("explicit paths should win");

    assert_eq!(path, PathBuf::from("/tmp/acp-test.json"));
}

#[test]
fn recent_sessions_path_falls_back_to_the_local_data_directory() {
    let path = recent_sessions_path_from(None, Some(PathBuf::from("/tmp/local-data")))
        .expect("fallback data dir should work");

    assert_eq!(
        path,
        PathBuf::from("/tmp/local-data/acp-orchestrator/recent-sessions.json")
    );
}

#[test]
fn recent_sessions_path_requires_a_data_directory_when_no_override_is_set() {
    let error = recent_sessions_path_from(None, None).expect_err("missing data dir should fail");

    assert!(matches!(error, CliError::MissingRecentSessionDirectory));
}

#[test]
fn create_recent_sessions_parent_skips_paths_without_a_directory_component() {
    create_recent_sessions_parent(Path::new(""))
        .expect("empty paths should not require directory creation");
}

async fn spawn_raw_http_server(response: &'static str) -> String {
    spawn_raw_http_server_bytes(response.as_bytes().to_vec()).await
}

async fn spawn_raw_http_server_bytes(payload: Vec<u8>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("server should bind");
    let address = listener
        .local_addr()
        .expect("server address should be readable");

    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("server should accept");
        stream
            .write_all(&payload)
            .await
            .expect("response should write");
    });

    format!("http://{address}")
}
