use super::*;

use acp_app_support::unique_temp_json_path;
use acp_contracts::{CloseSessionResponse, SessionHistoryResponse};
use std::{fs, sync::OnceLock};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::Mutex,
};

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn env_lock() -> &'static Mutex<()> {
    ENV_LOCK.get_or_init(|| Mutex::new(()))
}

#[tokio::test]
async fn run_chat_with_ui_records_recent_sessions_before_launching_the_ui() {
    let recent_path = unique_temp_json_path("acp-cli", "run-chat-with-ui");
    with_recent_sessions_path(&recent_path, async {
        let server_url = spawn_ordered_http_server(vec![json_response(
            &serde_json::to_vec(&CreateSessionResponse {
                session: SessionSnapshot {
                    id: "s_new".to_string(),
                    status: acp_contracts::SessionStatus::Active,
                    latest_sequence: 1,
                    messages: Vec::new(),
                    pending_permissions: Vec::new(),
                },
            })
            .expect("session response should serialize"),
        )])
        .await;
        let (tx, rx) = tokio::sync::oneshot::channel();

        run_chat_with_ui(
            ChatArgs {
                new: true,
                session_id: None,
                server_url: Some(server_url.clone()),
                auth_token: "developer".to_string(),
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
        assert!(
            fs::read_to_string(&recent_path)
                .expect("recent sessions should be written")
                .contains("s_new")
        );
    })
    .await;
}

#[tokio::test]
async fn run_chat_with_handlers_uses_the_noninteractive_repl_path() {
    let recent_path = unique_temp_json_path("acp-cli", "run-chat-with-handlers");
    with_recent_sessions_path(&recent_path, async {
        let server_url = spawn_ordered_http_server(vec![
            json_response(
                &serde_json::to_vec(&CreateSessionResponse {
                    session: SessionSnapshot {
                        id: "s_line".to_string(),
                        status: acp_contracts::SessionStatus::Active,
                        latest_sequence: 1,
                        messages: Vec::new(),
                        pending_permissions: Vec::new(),
                    },
                })
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
                server_url: Some(server_url.clone()),
                auth_token: "developer".to_string(),
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
    })
    .await;
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

    let chat_session = load_chat_session(&client, &server_url, &resumed_chat_args(&server_url))
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
                    status: acp_contracts::SessionStatus::Active,
                    latest_sequence: 2,
                    messages: vec![acp_contracts::ConversationMessage {
                        id: "m_snapshot".to_string(),
                        role: MessageRole::Assistant,
                        text: "from snapshot".to_string(),
                        created_at: chrono::Utc::now(),
                    }],
                    pending_permissions: Vec::new(),
                },
            })
            .expect("session response should serialize"),
        ),
        json_error_response("404 Not Found", "session not found"),
    ])
    .await;
    let client = Client::builder().build().expect("client should build");

    let chat_session = load_chat_session(&client, &server_url, &resumed_chat_args(&server_url))
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

    let error = load_chat_session(&client, &server_url, &resumed_chat_args(&server_url))
        .await
        .expect_err("unexpected history failures should surface");

    assert!(matches!(
        error,
        CliError::HttpStatus { status, message, .. }
            if status == StatusCode::INTERNAL_SERVER_ERROR && message == "history unavailable"
    ));
}

#[tokio::test]
async fn load_chat_session_prunes_stale_recent_sessions_when_sessions_are_missing() {
    let recent_path = unique_temp_json_path("acp-cli", "stale-resume");
    with_recent_sessions_path(&recent_path, async {
        let server_url = spawn_ordered_http_server(vec![json_error_response(
            "404 Not Found",
            "session not found",
        )])
        .await;
        record_recent_session(&RecentSessionEntry::new(
            "s_resume",
            &server_url,
            Utc::now(),
        ))
        .expect("recent session should record");
        let client = Client::builder().build().expect("client should build");

        let error = load_chat_session(&client, &server_url, &resumed_chat_args(&server_url))
            .await
            .expect_err("missing sessions should fail");

        assert!(matches!(
            error,
            CliError::HttpStatus { status, .. } if status == StatusCode::NOT_FOUND
        ));
        assert!(
            !fs::read_to_string(&recent_path)
                .expect("recent sessions should be readable")
                .contains("s_resume")
        );
    })
    .await;
}

#[tokio::test]
async fn run_session_list_and_close_cover_in_process_session_commands() {
    let recent_path = unique_temp_json_path("acp-cli", "run-session-commands");
    with_recent_sessions_path(&recent_path, async {
        record_recent_session(&RecentSessionEntry::new(
            "s_close",
            "http://127.0.0.1:8080",
            Utc::now(),
        ))
        .expect("recent session should record");

        run_session(SessionArgs {
            command: SessionCommand::List,
        })
        .await
        .expect("session list should succeed");

        let server_url = spawn_ordered_http_server(vec![json_response(
            &serde_json::to_vec(&CloseSessionResponse {
                session: SessionSnapshot {
                    id: "s_close".to_string(),
                    status: acp_contracts::SessionStatus::Closed,
                    latest_sequence: 3,
                    messages: Vec::new(),
                    pending_permissions: Vec::new(),
                },
            })
            .expect("close response should serialize"),
        )])
        .await;

        run_session(SessionArgs {
            command: SessionCommand::Close(CloseArgs {
                session_id: "s_close".to_string(),
                server_url: Some(server_url),
                auth_token: "developer".to_string(),
            }),
        })
        .await
        .expect("session close should succeed");

        let contents =
            fs::read_to_string(&recent_path).expect("recent sessions should be readable");
        assert!(!contents.contains("s_close"));
    })
    .await;
}

#[tokio::test]
async fn filter_recent_sessions_for_current_backend_only_keeps_current_resumable_entries() {
    let recent_path = unique_temp_json_path("acp-cli", "filter-session-list");
    with_recent_sessions_path(&recent_path, async {
        let server_url = spawn_ordered_http_server(vec![json_response(
            &serde_json::to_vec(&resumed_session_response())
                .expect("session response should serialize"),
        )])
        .await;
        record_recent_session(&RecentSessionEntry::new(
            "s_resume",
            &server_url,
            Utc::now(),
        ))
        .expect("current recent session should record");
        record_recent_session_now("s_other", "http://127.0.0.1:9999");

        let filtered = filter_recent_sessions_for_current_backend_with_env(&server_url).await;

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].session_id, "s_resume");
    })
    .await;
}

#[tokio::test]
async fn filter_recent_sessions_for_current_backend_prunes_stale_entries() {
    let recent_path = unique_temp_json_path("acp-cli", "filter-session-list-stale");
    with_recent_sessions_path(&recent_path, async {
        let server_url = spawn_ordered_http_server(vec![json_error_response(
            "404 Not Found",
            "session not found",
        )])
        .await;
        record_recent_session_now("s_stale", &server_url);

        let filtered = filter_recent_sessions_for_current_backend_with_env(&server_url).await;

        assert!(filtered.is_empty());
        assert!(
            !fs::read_to_string(&recent_path)
                .expect("recent sessions should be readable")
                .contains("s_stale")
        );
    })
    .await;
}

#[tokio::test]
async fn filter_recent_sessions_for_current_backend_keeps_entries_on_non_404_errors() {
    let recent_path = unique_temp_json_path("acp-cli", "filter-session-list-error");
    with_recent_sessions_path(&recent_path, async {
        let server_url = spawn_ordered_http_server(vec![json_error_response(
            "500 Internal Server Error",
            "backend unavailable",
        )])
        .await;
        record_recent_session_now("s_degraded", &server_url);

        let filtered = filter_recent_sessions_for_current_backend_with_env(&server_url).await;

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].session_id, "s_degraded");
        assert!(
            fs::read_to_string(&recent_path)
                .expect("recent sessions should be readable")
                .contains("s_degraded")
        );
    })
    .await;
}

fn record_recent_session_now(session_id: &str, server_url: &str) {
    record_recent_session(&RecentSessionEntry::new(session_id, server_url, Utc::now()))
        .expect("recent session should record");
}

async fn filter_recent_sessions_for_current_backend_with_env(
    server_url: &str,
) -> Vec<RecentSessionEntry> {
    unsafe {
        std::env::set_var("ACP_SERVER_URL", server_url);
        std::env::set_var("ACP_AUTH_TOKEN", "developer");
    }

    filter_recent_sessions_for_current_backend(
        load_recent_sessions().expect("recent sessions should load"),
    )
    .await
    .expect("current backend filtering should succeed")
}

async fn with_recent_sessions_path<Fut>(path: &std::path::Path, action: Fut)
where
    Fut: std::future::Future<Output = ()>,
{
    let _guard = env_lock().lock().await;
    let previous_recent_path = std::env::var_os("ACP_RECENT_SESSIONS_PATH");
    let previous_server_url = std::env::var_os("ACP_SERVER_URL");
    let previous_auth_token = std::env::var_os("ACP_AUTH_TOKEN");
    unsafe { std::env::set_var("ACP_RECENT_SESSIONS_PATH", path) };
    unsafe {
        std::env::remove_var("ACP_SERVER_URL");
        std::env::remove_var("ACP_AUTH_TOKEN");
    }
    action.await;
    match previous_recent_path {
        Some(value) => unsafe { std::env::set_var("ACP_RECENT_SESSIONS_PATH", value) },
        None => unsafe { std::env::remove_var("ACP_RECENT_SESSIONS_PATH") },
    }
    match previous_server_url {
        Some(value) => unsafe { std::env::set_var("ACP_SERVER_URL", value) },
        None => unsafe { std::env::remove_var("ACP_SERVER_URL") },
    }
    match previous_auth_token {
        Some(value) => unsafe { std::env::set_var("ACP_AUTH_TOKEN", value) },
        None => unsafe { std::env::remove_var("ACP_AUTH_TOKEN") },
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

fn resumed_chat_args(server_url: &str) -> ChatArgs {
    ChatArgs {
        new: false,
        session_id: Some("s_resume".to_string()),
        server_url: Some(server_url.to_string()),
        auth_token: "developer".to_string(),
    }
}

fn resumed_history_response() -> SessionHistoryResponse {
    SessionHistoryResponse {
        session_id: "s_resume".to_string(),
        messages: vec![acp_contracts::ConversationMessage {
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
            status: acp_contracts::SessionStatus::Active,
            latest_sequence: 2,
            messages: Vec::new(),
            pending_permissions: vec![acp_contracts::PermissionRequest {
                request_id: "req_1".to_string(),
                summary: "read_text_file README.md".to_string(),
            }],
        },
    }
}
