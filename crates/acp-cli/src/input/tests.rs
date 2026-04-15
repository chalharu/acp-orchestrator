use super::*;

use acp_contracts::SlashCompletionsResponse;
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::oneshot,
};

#[test]
fn completion_start_replaces_only_the_current_argument() {
    assert_eq!(completion_start("/ap"), 0);
    assert_eq!(completion_start("/approve req_"), 9);
}

#[test]
fn completion_query_only_allows_supported_slash_shapes() {
    assert_eq!(completion_query("/ap", 3), Some("/ap"));
    assert_eq!(completion_query("/approve req_", 13), Some("/approve req_"));
    assert_eq!(completion_query("/home/alice", 11), None);
    assert_eq!(completion_query("/quit now", 9), None);
    assert_eq!(completion_query("hello", 5), None);
}

#[tokio::test]
async fn slash_completion_helper_queries_the_backend_for_command_candidates() {
    let (url, request_line_rx) = spawn_completion_server(SlashCompletionsResponse {
        candidates: vec![CompletionCandidate {
            label: "/approve <request-id>".to_string(),
            insert_text: "/approve ".to_string(),
            detail: "Approve a pending permission request".to_string(),
            kind: CompletionKind::Command,
        }],
    })
    .await;
    let helper = SlashCompletionHelper::new(
        Handle::current(),
        Client::builder().build().expect("client should build"),
        url,
        "developer".to_string(),
        "s_test".to_string(),
    );

    let (start, candidates) = tokio::task::spawn_blocking(move || {
        let history = DefaultHistory::new();
        let context = Context::new(&history);
        helper.complete("/ap", 3, &context)
    })
    .await
    .expect("completion worker should join")
    .expect("completion query should succeed");

    assert_eq!(start, 0);
    assert_eq!(candidates[0].replacement, "/approve ");
    assert!(candidates[0].display.contains("/approve <request-id>"));
    let request_line = request_line_rx
        .await
        .expect("request line should be captured");
    assert!(request_line.contains("/api/v1/completions/slash?sessionId=s_test&prefix=%2Fap"));
}

#[tokio::test]
async fn slash_completion_helper_replaces_only_permission_request_ids() {
    let (url, _) = spawn_completion_server(SlashCompletionsResponse {
        candidates: vec![CompletionCandidate {
            label: "req_1".to_string(),
            insert_text: "req_1".to_string(),
            detail: "read_text_file README.md".to_string(),
            kind: CompletionKind::Parameter,
        }],
    })
    .await;
    let helper = SlashCompletionHelper::new(
        Handle::current(),
        Client::builder().build().expect("client should build"),
        url,
        "developer".to_string(),
        "s_test".to_string(),
    );

    let (start, candidates) = tokio::task::spawn_blocking(move || {
        let history = DefaultHistory::new();
        let context = Context::new(&history);
        helper.complete("/approve req_", 13, &context)
    })
    .await
    .expect("completion worker should join")
    .expect("parameter completion should succeed");

    assert_eq!(start, 9);
    assert_eq!(candidates[0].replacement, "req_1");
}

#[tokio::test]
async fn slash_completion_helper_ignores_non_slash_input() {
    let helper = SlashCompletionHelper::new(
        Handle::current(),
        Client::builder().build().expect("client should build"),
        "http://127.0.0.1:9".to_string(),
        "developer".to_string(),
        "s_test".to_string(),
    );

    let (start, candidates) = tokio::task::spawn_blocking(move || {
        let history = DefaultHistory::new();
        let context = Context::new(&history);
        helper.complete("hello", 5, &context)
    })
    .await
    .expect("completion worker should join")
    .expect("non-slash input should be ignored");

    assert_eq!(start, 5);
    assert!(candidates.is_empty());
}

#[tokio::test]
async fn slash_completion_helper_times_out_when_the_backend_stalls() {
    let url = spawn_stalled_completion_server().await;
    let helper = SlashCompletionHelper::with_timeout(
        Handle::current(),
        Client::builder().build().expect("client should build"),
        url,
        "developer".to_string(),
        "s_test".to_string(),
        Duration::from_millis(50),
    );

    let (start, candidates) = tokio::task::spawn_blocking(move || {
        let history = DefaultHistory::new();
        let context = Context::new(&history);
        helper.complete("/ap", 3, &context)
    })
    .await
    .expect("completion worker should join")
    .expect("timed-out completions should degrade cleanly");

    assert_eq!(start, 3);
    assert!(candidates.is_empty());
}

async fn spawn_completion_server(
    response: SlashCompletionsResponse,
) -> (String, oneshot::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("completion server should bind");
    let address = listener
        .local_addr()
        .expect("completion server address should be readable");
    let payload = serde_json::to_vec(&response).expect("response should serialize");
    let (request_line_tx, request_line_rx) = oneshot::channel();

    tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("completion server should accept");
        let mut buffer = [0u8; 1024];
        let bytes_read = stream
            .read(&mut buffer)
            .await
            .expect("completion server should read the request");
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
            .expect("completion response headers should write");
        stream
            .write_all(&payload)
            .await
            .expect("completion response body should write");
    });

    (format!("http://{address}"), request_line_rx)
}

async fn spawn_stalled_completion_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("stalled completion server should bind");
    let address = listener
        .local_addr()
        .expect("stalled completion server address should be readable");

    tokio::spawn(async move {
        let (_stream, _) = listener
            .accept()
            .await
            .expect("stalled completion server should accept");
        tokio::time::sleep(Duration::from_millis(200)).await;
    });

    format!("http://{address}")
}
