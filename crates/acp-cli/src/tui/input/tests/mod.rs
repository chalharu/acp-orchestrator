use super::*;

use acp_contracts::{
    CompletionCandidate, CompletionKind, CreateSessionResponse, PermissionRequest, PromptResponse,
    SessionSnapshot, SessionStatus, SlashCompletionsResponse,
};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

mod completion;
mod events;
mod outcomes;
mod submission;

fn command_candidate(label: &str, insert_text: &str, detail: &str) -> CompletionCandidate {
    CompletionCandidate {
        label: label.to_string(),
        insert_text: insert_text.to_string(),
        detail: detail.to_string(),
        kind: CompletionKind::Command,
    }
}

fn base_app() -> ChatApp {
    ChatApp::new("s_test", "http://127.0.0.1:8080", false, &[], &[], vec![])
}

fn build_context<'a>(
    handle: &'a Handle,
    client: &'a Client,
    server_url: &'a str,
    auth_token: &'a str,
    session_id: &'a str,
) -> UiContext<'a> {
    UiContext {
        runtime_handle: handle,
        client,
        server_url,
        auth_token,
        session_id,
    }
}

fn active_session(requests: Vec<PermissionRequest>) -> CreateSessionResponse {
    CreateSessionResponse {
        session: SessionSnapshot {
            id: "s_test".to_string(),
            status: SessionStatus::Active,
            latest_sequence: 1,
            messages: Vec::new(),
            pending_permissions: requests,
        },
    }
}

async fn spawn_completion_server(
    response: SlashCompletionsResponse,
) -> (String, tokio::sync::oneshot::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("completion server should bind");
    let address = listener
        .local_addr()
        .expect("completion server address should be readable");
    let payload = serde_json::to_vec(&response).expect("response should serialize");
    let (request_line_tx, request_line_rx) = tokio::sync::oneshot::channel();

    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("server should accept");
        let mut buffer = [0u8; 1024];
        let bytes_read = stream.read(&mut buffer).await.expect("request should read");
        let request = String::from_utf8_lossy(&buffer[..bytes_read]);
        let request_line = request.lines().next().unwrap_or_default().to_string();
        let _ = request_line_tx.send(request_line);
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            payload.len()
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("response headers should write");
        stream
            .write_all(&payload)
            .await
            .expect("response body should write");
    });

    (format!("http://{address}"), request_line_rx)
}

async fn spawn_prompt_server() -> (
    String,
    tokio::sync::oneshot::Receiver<String>,
    tokio::sync::oneshot::Receiver<String>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("prompt server should bind");
    let address = listener
        .local_addr()
        .expect("prompt server address should be readable");
    let (request_line_tx, request_line_rx) = tokio::sync::oneshot::channel();
    let (request_body_tx, request_body_rx) = tokio::sync::oneshot::channel();

    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("server should accept");
        let mut buffer = [0u8; 2048];
        let bytes_read = stream.read(&mut buffer).await.expect("request should read");
        let request = String::from_utf8_lossy(&buffer[..bytes_read]).to_string();
        let request_line = request.lines().next().unwrap_or_default().to_string();
        let body = request
            .split("\r\n\r\n")
            .nth(1)
            .unwrap_or_default()
            .to_string();
        let _ = request_line_tx.send(request_line);
        let _ = request_body_tx.send(body);
        let payload = serde_json::to_vec(&PromptResponse { accepted: true })
            .expect("prompt response should serialize");
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            payload.len()
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("response headers should write");
        stream
            .write_all(&payload)
            .await
            .expect("response body should write");
    });

    (
        format!("http://{address}"),
        request_line_rx,
        request_body_rx,
    )
}

async fn spawn_session_server(
    response: CreateSessionResponse,
) -> (String, tokio::sync::oneshot::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("session server should bind");
    let address = listener
        .local_addr()
        .expect("session server address should be readable");
    let payload = serde_json::to_vec(&response).expect("response should serialize");
    let (request_line_tx, request_line_rx) = tokio::sync::oneshot::channel();

    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("server should accept");
        let mut buffer = [0u8; 1024];
        let bytes_read = stream.read(&mut buffer).await.expect("request should read");
        let request = String::from_utf8_lossy(&buffer[..bytes_read]);
        let request_line = request.lines().next().unwrap_or_default().to_string();
        let _ = request_line_tx.send(request_line);
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            payload.len()
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("response headers should write");
        stream
            .write_all(&payload)
            .await
            .expect("response body should write");
    });

    (format!("http://{address}"), request_line_rx)
}

async fn spawn_stalled_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("stalled server should bind");
    let address = listener
        .local_addr()
        .expect("stalled server address should be readable");

    tokio::spawn(async move {
        let (_stream, _) = listener.accept().await.expect("server should accept");
        tokio::time::sleep(SLASH_COMPLETION_TIMEOUT + std::time::Duration::from_millis(50)).await;
    });

    format!("http://{address}")
}
