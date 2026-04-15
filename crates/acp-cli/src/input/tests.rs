use super::*;

use std::io::{self, BufRead, Cursor, Write};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::oneshot,
};

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

#[tokio::test]
async fn execute_repl_line_propagates_prompt_failures() {
    let client = Client::builder().build().expect("client should build");

    let error = execute_repl_line(
        &client,
        "http://127.0.0.1:9",
        "developer",
        "s_test",
        "hello",
    )
    .await
    .expect_err("unreachable backends should surface prompt failures");

    assert!(matches!(error, crate::CliError::SendRequest { .. }));
}

#[test]
fn read_prompt_line_from_returns_a_line_and_prints_the_prompt() {
    let mut reader = Cursor::new(b"hello\n".to_vec());
    let mut writer = Vec::new();

    let line =
        read_prompt_line_from(&mut reader, &mut writer).expect("prompt reads should succeed");

    assert_eq!(line.as_deref(), Some("hello\n"));
    assert_eq!(writer, b"> ");
}

#[test]
fn read_prompt_line_from_returns_none_at_eof() {
    let mut reader = Cursor::new(Vec::<u8>::new());
    let mut writer = Vec::new();

    let line = read_prompt_line_from(&mut reader, &mut writer).expect("eof should be handled");

    assert_eq!(line, None);
    assert_eq!(writer, b"> ");
}

#[test]
fn read_prompt_line_from_surfaces_prompt_flush_failures() {
    let mut reader = Cursor::new(b"hello\n".to_vec());
    let mut writer = FlushFailureWriter::default();

    let error =
        read_prompt_line_from(&mut reader, &mut writer).expect_err("flush errors should surface");

    assert!(matches!(error, crate::CliError::FlushPrompt { .. }));
}

#[test]
fn read_prompt_line_from_surfaces_read_failures() {
    let mut reader = ReadFailureReader;
    let mut writer = Vec::new();

    let error =
        read_prompt_line_from(&mut reader, &mut writer).expect_err("read errors should surface");

    assert!(matches!(error, crate::CliError::ReadPromptLine { .. }));
}

#[derive(Default)]
struct FlushFailureWriter(Vec<u8>);

impl Write for FlushFailureWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Err(io::Error::other("flush failed"))
    }
}

struct ReadFailureReader;

impl io::Read for ReadFailureReader {
    fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::other("read failed"))
    }
}

impl BufRead for ReadFailureReader {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        Err(io::Error::other("read failed"))
    }

    fn consume(&mut self, _amt: usize) {}
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
