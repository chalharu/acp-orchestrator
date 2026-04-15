use super::*;

use acp_contracts::{CompletionCandidate, CompletionKind, SlashCompletionsResponse};
use std::{
    collections::VecDeque,
    io,
    sync::{Arc, Mutex},
    time::Duration,
};
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
    let helper = build_completion_helper(&url);
    let (start, candidates) = complete_with_helper(helper, "/ap", 3).await;

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
    let helper = build_completion_helper(&url);
    let (start, candidates) = complete_with_helper(helper, "/approve req_", 13).await;

    assert_eq!(start, 9);
    assert_eq!(candidates[0].replacement, "req_1");
}

#[tokio::test]
async fn slash_completion_helper_ignores_non_slash_input() {
    let helper = build_completion_helper("http://127.0.0.1:9");
    let (start, candidates) = complete_with_helper(helper, "hello", 5).await;

    assert_eq!(start, 5);
    assert!(candidates.is_empty());
}

#[tokio::test]
async fn slash_completion_helper_ignores_backend_failures() {
    let helper = build_completion_helper("http://127.0.0.1:9");
    let (start, candidates) = complete_with_helper(helper, "/ap", 3).await;

    assert_eq!(start, 3);
    assert!(candidates.is_empty());
}

#[tokio::test]
async fn slash_completion_helper_times_out_when_the_backend_stalls() {
    let url = spawn_stalled_completion_server().await;
    let helper = build_completion_helper_with_timeout(&url, Duration::from_millis(50));
    let (start, candidates) = complete_with_helper(helper, "/ap", 3).await;

    assert_eq!(start, 3);
    assert!(candidates.is_empty());
}

#[tokio::test]
async fn execute_repl_line_submits_trimmed_prompts() {
    let (url, request_line_rx, request_body_rx) = spawn_prompt_server().await;
    let client = Client::builder().build().expect("client should build");

    let should_quit = execute_repl_line(&client, &url, "developer", "s_test", "  hello  ")
        .await
        .expect("prompt submission should succeed");

    assert!(!should_quit);
    assert_eq!(
        request_line_rx
            .await
            .expect("request line should be captured"),
        "POST /api/v1/sessions/s_test/messages HTTP/1.1"
    );
    assert_eq!(
        request_body_rx
            .await
            .expect("request body should be captured"),
        "{\"text\":\"hello\"}"
    );
}

#[tokio::test]
async fn execute_repl_line_routes_slash_commands() {
    let client = Client::builder().build().expect("client should build");

    let should_quit = execute_repl_line(
        &client,
        "http://127.0.0.1:9",
        "developer",
        "s_test",
        "/quit",
    )
    .await
    .expect("slash commands should stay in-process");

    assert!(should_quit);
}

#[tokio::test]
async fn execute_repl_line_ignores_blank_input() {
    let client = Client::builder().build().expect("client should build");

    let should_quit =
        execute_repl_line(&client, "http://127.0.0.1:9", "developer", "s_test", "   ")
            .await
            .expect("blank input should be ignored");

    assert!(!should_quit);
}

#[test]
fn drive_editor_repl_records_history_and_stops_after_quit() {
    let (result, history) = run_fake_editor(vec![Ok("/quit".to_string())], "http://127.0.0.1:9");
    result.expect("quit should exit the editor loop");

    assert_eq!(
        history
            .lock()
            .expect("history should not poison")
            .as_slice(),
        ["/quit"]
    );
}

#[test]
fn drive_editor_repl_continues_after_non_quit_commands() {
    let (result, history) = run_fake_editor(
        vec![Ok("/unknown".to_string()), Err(ReadlineError::Eof)],
        "http://127.0.0.1:9",
    );
    result.expect("non-quit commands should continue the editor loop");

    assert_eq!(
        history
            .lock()
            .expect("history should not poison")
            .as_slice(),
        ["/unknown"]
    );
}

#[test]
fn drive_editor_repl_skips_blank_lines_and_handles_interrupts() {
    let (result, history) = run_fake_editor(
        vec![
            Ok("   ".to_string()),
            Err(ReadlineError::Interrupted),
            Err(ReadlineError::Eof),
        ],
        "http://127.0.0.1:9",
    );
    result.expect("interrupts should stay in the editor loop");

    assert!(
        history
            .lock()
            .expect("history should not poison")
            .is_empty()
    );
}

#[test]
fn drive_editor_repl_surfaces_readline_errors() {
    let (result, _) = run_fake_editor(
        vec![Err(ReadlineError::Io(io::Error::other("boom")))],
        "http://127.0.0.1:9",
    );
    let error = result.expect_err("unexpected readline errors should surface");

    assert!(matches!(
        error,
        crate::CliError::ReadInteractivePrompt { .. }
    ));
}

#[test]
fn drive_editor_repl_propagates_command_errors() {
    let (result, _) = run_fake_editor(vec![Ok("hello".to_string())], "http://127.0.0.1:9");
    let error = result.expect_err("command failures should propagate");

    assert!(matches!(error, crate::CliError::SendRequest { .. }));
}

#[test]
fn read_prompt_line_from_returns_a_line_and_prints_the_prompt() {
    let mut reader = io::Cursor::new(b"hello\n".to_vec());
    let mut writer = Vec::new();

    let line =
        read_prompt_line_from(&mut reader, &mut writer).expect("prompt reads should succeed");

    assert_eq!(line.as_deref(), Some("hello\n"));
    assert_eq!(writer, b"> ");
}

#[test]
fn read_prompt_line_from_returns_none_at_eof() {
    let mut reader = io::Cursor::new(Vec::<u8>::new());
    let mut writer = Vec::new();

    let line = read_prompt_line_from(&mut reader, &mut writer).expect("eof should be handled");

    assert_eq!(line, None);
    assert_eq!(writer, b"> ");
}

#[test]
fn completion_display_formats_candidate_kind_and_detail() {
    let command = completion_display(&CompletionCandidate {
        label: "/help".to_string(),
        insert_text: "/help".to_string(),
        detail: "Show available slash commands".to_string(),
        kind: CompletionKind::Command,
    });
    let parameter = completion_display(&CompletionCandidate {
        label: "req_1".to_string(),
        insert_text: "req_1".to_string(),
        detail: "read_text_file README.md".to_string(),
        kind: CompletionKind::Parameter,
    });

    assert_eq!(command, "/help\tcommand\tShow available slash commands");
    assert_eq!(parameter, "req_1\tparameter\tread_text_file README.md");
}

#[test]
fn new_helpers_use_the_default_completion_timeout() {
    let runtime = tokio::runtime::Runtime::new().expect("runtime should build");
    let helper = SlashCompletionHelper::new(
        runtime.handle().clone(),
        Client::builder().build().expect("client should build"),
        "http://127.0.0.1:9".to_string(),
        "developer".to_string(),
        "s_test".to_string(),
    );

    assert_eq!(helper.completion_timeout, SLASH_COMPLETION_TIMEOUT);
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

async fn spawn_prompt_server() -> (String, oneshot::Receiver<String>, oneshot::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("prompt server should bind");
    let address = listener
        .local_addr()
        .expect("prompt server address should be readable");
    let (request_line_tx, request_line_rx) = oneshot::channel();
    let (request_body_tx, request_body_rx) = oneshot::channel();

    tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("prompt server should accept");
        let mut buffer = [0u8; 2048];
        let bytes_read = stream
            .read(&mut buffer)
            .await
            .expect("prompt server should read the request");
        let request = String::from_utf8_lossy(&buffer[..bytes_read]);
        let request_line = request.lines().next().unwrap_or_default().to_string();
        let request_body = request
            .split("\r\n\r\n")
            .nth(1)
            .unwrap_or_default()
            .to_string();
        let _ = request_line_tx.send(request_line);
        let _ = request_body_tx.send(request_body);
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 17\r\nConnection: close\r\n\r\n{\"accepted\":true}",
            )
            .await
            .expect("prompt response should write");
    });

    (
        format!("http://{address}"),
        request_line_rx,
        request_body_rx,
    )
}

fn build_completion_helper(base_url: &str) -> SlashCompletionHelper {
    build_completion_helper_with_timeout(base_url, SLASH_COMPLETION_TIMEOUT)
}

fn build_completion_helper_with_timeout(
    base_url: &str,
    completion_timeout: Duration,
) -> SlashCompletionHelper {
    SlashCompletionHelper::with_timeout(
        Handle::current(),
        Client::builder().build().expect("client should build"),
        base_url.to_string(),
        "developer".to_string(),
        "s_test".to_string(),
        completion_timeout,
    )
}

async fn complete_with_helper(
    helper: SlashCompletionHelper,
    line: &'static str,
    position: usize,
) -> (usize, Vec<Pair>) {
    tokio::task::spawn_blocking(move || {
        let history = DefaultHistory::new();
        let context = Context::new(&history);
        helper.complete(line, position, &context)
    })
    .await
    .expect("completion worker should join")
    .expect("completion query should succeed")
}

fn run_fake_editor(
    results: Vec<std::result::Result<String, ReadlineError>>,
    base_url: &str,
) -> (Result<()>, Arc<Mutex<Vec<String>>>) {
    let runtime = tokio::runtime::Runtime::new().expect("runtime should build");
    let client = Client::builder().build().expect("client should build");
    let history = Arc::new(Mutex::new(Vec::new()));
    let mut editor = FakeEditor::new(results, history.clone());
    let result = drive_editor_repl(
        &mut editor,
        runtime.handle(),
        &client,
        base_url,
        "developer",
        "s_test",
    );
    (result, history)
}

struct FakeEditor {
    results: VecDeque<std::result::Result<String, ReadlineError>>,
    history: Arc<Mutex<Vec<String>>>,
}

impl FakeEditor {
    fn new(
        results: Vec<std::result::Result<String, ReadlineError>>,
        history: Arc<Mutex<Vec<String>>>,
    ) -> Self {
        Self {
            results: results.into(),
            history,
        }
    }
}

impl PromptEditor for FakeEditor {
    fn readline(&mut self, _prompt: &str) -> std::result::Result<String, ReadlineError> {
        self.results
            .pop_front()
            .expect("fake editor should have a queued readline result")
    }

    fn add_history_entry(&mut self, line: &str) {
        self.history
            .lock()
            .expect("history should not poison")
            .push(line.to_string());
    }
}
