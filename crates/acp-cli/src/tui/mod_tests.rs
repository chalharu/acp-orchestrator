use super::*;

use acp_contracts::{
    CompletionCandidate, CompletionKind, MessageRole, SessionSnapshot, SessionStatus,
};
use chrono::Utc;
use std::sync::OnceLock;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::Mutex,
    time::Duration,
};

static RUNNER_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn runner_lock() -> &'static Mutex<()> {
    RUNNER_LOCK.get_or_init(|| Mutex::new(()))
}

fn chat_session() -> ChatSession {
    ChatSession {
        session: SessionSnapshot {
            id: "s_test".to_string(),
            status: SessionStatus::Active,
            latest_sequence: 1,
            messages: Vec::new(),
            pending_permissions: Vec::new(),
        },
        resume_history: Vec::new(),
        resumed: false,
    }
}

fn resumed_chat_session() -> ChatSession {
    ChatSession {
        session: SessionSnapshot {
            id: "s_test".to_string(),
            status: SessionStatus::Active,
            latest_sequence: 2,
            messages: Vec::new(),
            pending_permissions: vec![acp_contracts::PermissionRequest {
                request_id: "req_1".to_string(),
                summary: "read_text_file README.md".to_string(),
            }],
        },
        resume_history: vec![acp_contracts::ConversationMessage {
            id: "m_1".to_string(),
            role: MessageRole::Assistant,
            text: "resumed".to_string(),
            created_at: Utc::now(),
        }],
        resumed: true,
    }
}

fn command_candidate() -> CompletionCandidate {
    CompletionCandidate {
        label: "/help".to_string(),
        insert_text: "/help".to_string(),
        detail: "Show help".to_string(),
        kind: CompletionKind::Command,
    }
}

#[tokio::test]
async fn run_chat_tui_uses_the_configured_terminal_runner() {
    let _guard = runner_lock().lock().await;
    let (url, server_task) = spawn_tui_server(
        Some(serde_json::json!({
            "candidates": [command_candidate()],
        })),
        false,
    )
    .await;
    let client = Client::builder().build().expect("client should build");

    set_terminal_ui_runner_override(Some(|_handle, _state| Ok(())));
    let result = run_chat_tui(client, url, "developer".to_string(), resumed_chat_session()).await;
    set_terminal_ui_runner_override(None);

    server_task.abort();
    result.expect("override runner should let run_chat_tui finish");
}

#[test]
fn terminal_ui_runner_defaults_to_the_runtime_runner() {
    set_terminal_ui_runner_override(None);
    assert!(std::ptr::fn_addr_eq(
        terminal_ui_runner(),
        runtime::run_terminal_ui as TerminalUiRunner,
    ));
}

#[tokio::test]
async fn prepare_startup_state_with_timeout_loads_catalog() {
    let (url, server_task) = spawn_tui_server(
        Some(serde_json::json!({
            "candidates": [command_candidate()],
        })),
        false,
    )
    .await;
    let client = Client::builder().build().expect("client should build");

    let startup = prepare_startup_state_with_timeout(
        &client,
        &url,
        "developer",
        &chat_session(),
        Duration::from_millis(200),
    )
    .await;

    server_task.abort();
    assert_eq!(startup.command_catalog[0].label, "/help");
    assert!(startup.startup_statuses.is_empty());
}

#[tokio::test]
async fn prepare_startup_state_with_timeout_records_backend_errors() {
    let (url, server_task) = spawn_error_server(error_json_response(
        "500 Internal Server Error",
        "catalog unavailable",
    ))
    .await;
    let client = Client::builder().build().expect("client should build");

    let startup = prepare_startup_state_with_timeout(
        &client,
        &url,
        "developer",
        &chat_session(),
        Duration::from_millis(200),
    )
    .await;

    server_task.abort();
    assert!(startup.command_catalog.is_empty());
    assert!(
        startup
            .startup_statuses
            .iter()
            .any(|status| status.contains("catalog unavailable"))
    );
}

#[tokio::test]
async fn prepare_startup_state_with_timeout_records_stalls() {
    let (url, server_task) = spawn_tui_server(None, true).await;
    let client = Client::builder().build().expect("client should build");

    let startup = prepare_startup_state_with_timeout(
        &client,
        &url,
        "developer",
        &chat_session(),
        Duration::from_millis(50),
    )
    .await;

    server_task.abort();
    assert!(startup.command_catalog.is_empty());
    assert_eq!(
        startup.startup_statuses,
        vec!["slash command catalog timed out".to_string()]
    );
}

#[tokio::test]
async fn spawn_stream_task_forwards_updates_and_end_events() {
    let (url, server_task) = spawn_tui_server(None, false).await;
    let client = Client::builder().build().expect("client should build");
    let (event_tx, mut event_rx) = mpsc::unbounded_channel();
    let stream_task = spawn_stream_task(
        client,
        url,
        "developer".to_string(),
        &chat_session(),
        None,
        event_tx,
    );

    match event_rx.recv().await.expect("stream update should arrive") {
        TuiEvent::Stream(StreamUpdate::Status(message)) => assert_eq!(message, "working"),
        other => panic!("unexpected first event: {other:?}"),
    }
    match event_rx.recv().await.expect("stream end should arrive") {
        TuiEvent::StreamEnded(message) => assert_eq!(message, "event stream ended"),
        other => panic!("unexpected second event: {other:?}"),
    }

    stream_task.abort();
    server_task.abort();
}

#[tokio::test]
async fn spawn_stream_task_reports_stream_errors() {
    let (url, server_task) = spawn_error_server(error_json_response(
        "500 Internal Server Error",
        "stream failed",
    ))
    .await;
    let client = Client::builder().build().expect("client should build");
    let (event_tx, mut event_rx) = mpsc::unbounded_channel();
    let _stream_task = spawn_stream_task(
        client,
        url,
        "developer".to_string(),
        &chat_session(),
        None,
        event_tx,
    );

    match event_rx.recv().await.expect("stream end should arrive") {
        TuiEvent::StreamEnded(message) => {
            assert!(message.starts_with("event stream ended:"));
        }
        other => panic!("unexpected event: {other:?}"),
    }

    server_task.abort();
}

#[tokio::test]
async fn run_chat_tui_with_runner_returns_runner_results_without_a_real_terminal() {
    let (url, server_task) = spawn_tui_server(
        Some(serde_json::json!({
            "candidates": [command_candidate()],
        })),
        false,
    )
    .await;
    let client = Client::builder().build().expect("client should build");

    run_chat_tui_with_runner(
        client.clone(),
        url.clone(),
        "developer".to_string(),
        chat_session(),
        |_handle, _state| Ok(()),
    )
    .await
    .expect("runner success should propagate");

    let error = run_chat_tui_with_runner(
        client,
        url,
        "developer".to_string(),
        chat_session(),
        |_handle, _state| Err(crate::CliError::MissingRecentSessionDirectory),
    )
    .await
    .expect_err("runner failures should propagate");

    server_task.abort();
    assert!(matches!(
        error,
        crate::CliError::MissingRecentSessionDirectory
    ));
}

async fn spawn_tui_server(
    completion_response: Option<serde_json::Value>,
    stall_completion: bool,
) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("server should bind");
    let address = listener
        .local_addr()
        .expect("server address should be readable");

    let task = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.expect("server should accept");
            let completion_response = completion_response.clone();
            tokio::spawn(async move {
                let mut buffer = [0u8; 2048];
                let bytes_read = stream.read(&mut buffer).await.expect("request should read");
                let request = String::from_utf8_lossy(&buffer[..bytes_read]);
                let request_line = request.lines().next().unwrap_or_default().to_string();
                if request_line.contains("/api/v1/completions/slash") {
                    if stall_completion {
                        tokio::time::sleep(Duration::from_millis(200)).await;
                        return;
                    }
                    let payload = serde_json::to_vec(
                        &completion_response.expect("completion response should be configured"),
                    )
                    .expect("completion payload should serialize");
                    write_response(&mut stream, "application/json", &payload).await;
                    return;
                }
                if request_line.contains("/api/v1/sessions/s_test/events") {
                    let payload =
                        b"data: {\"sequence\":1,\"kind\":\"status\",\"message\":\"working\"}\n\n";
                    write_response(&mut stream, "text/event-stream", payload).await;
                }
            });
        }
    });

    (format!("http://{address}"), task)
}

async fn spawn_error_server(response: Vec<u8>) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("server should bind");
    let address = listener
        .local_addr()
        .expect("server address should be readable");

    let task = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.expect("server should accept");
            let mut buffer = [0u8; 1024];
            let _ = stream.read(&mut buffer).await;
            stream
                .write_all(&response)
                .await
                .expect("error response should write");
        }
    });

    (format!("http://{address}"), task)
}

fn error_json_response(status_line: &str, message: &str) -> Vec<u8> {
    let payload = serde_json::json!({ "error": message }).to_string();
    format!(
        "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
        payload.len()
    )
    .into_bytes()
}

async fn write_response(stream: &mut tokio::net::TcpStream, content_type: &str, payload: &[u8]) {
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    );
    stream
        .write_all(response.as_bytes())
        .await
        .expect("response headers should write");
    stream
        .write_all(payload)
        .await
        .expect("response body should write");
}
