use std::{path::PathBuf, process::Stdio, sync::Arc, time::Duration};

use acp_mock::support::runtime::read_startup_url;
use agent_client_protocol::{self as acp, schema};
use tokio::{net::TcpStream, process::Command, sync::Mutex, time::timeout};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Debug, Clone)]
struct TestClient {
    reply: Arc<Mutex<String>>,
    written_file: Arc<Mutex<Option<(String, String)>>>,
    killed_terminals: Arc<Mutex<Vec<String>>>,
}

impl TestClient {
    fn new() -> Self {
        Self {
            reply: Arc::new(Mutex::new(String::new())),
            written_file: Arc::new(Mutex::new(None)),
            killed_terminals: Arc::new(Mutex::new(Vec::new())),
        }
    }

    async fn reply_text(&self) -> String {
        self.reply.lock().await.clone()
    }

    async fn written_file(&self) -> Option<(String, String)> {
        self.written_file.lock().await.clone()
    }

    async fn killed_terminals(&self) -> Vec<String> {
        self.killed_terminals.lock().await.clone()
    }

    async fn request_permission(
        &self,
        args: schema::RequestPermissionRequest,
    ) -> acp::Result<schema::RequestPermissionResponse> {
        let selected = args
            .options
            .iter()
            .find(|option| {
                matches!(
                    option.kind,
                    schema::PermissionOptionKind::AllowOnce
                        | schema::PermissionOptionKind::AllowAlways
                )
            })
            .ok_or_else(acp::Error::invalid_params)?;
        Ok(schema::RequestPermissionResponse::new(
            schema::RequestPermissionOutcome::Selected(schema::SelectedPermissionOutcome::new(
                selected.option_id.to_string(),
            )),
        ))
    }

    async fn session_notification(&self, args: schema::SessionNotification) -> acp::Result<()> {
        if let schema::SessionUpdate::AgentMessageChunk(chunk) = args.update {
            self.reply
                .lock()
                .await
                .push_str(&content_text(chunk.content));
        }
        Ok(())
    }

    async fn read_text_file(
        &self,
        args: schema::ReadTextFileRequest,
    ) -> acp::Result<schema::ReadTextFileResponse> {
        if args.path != std::path::Path::new("/workspace/README.md") {
            return Err(acp::Error::invalid_params());
        }
        Ok(schema::ReadTextFileResponse::new("binary-readme"))
    }

    async fn write_text_file(
        &self,
        args: schema::WriteTextFileRequest,
    ) -> acp::Result<schema::WriteTextFileResponse> {
        *self.written_file.lock().await = Some((args.path.display().to_string(), args.content));
        Ok(schema::WriteTextFileResponse::new())
    }

    async fn create_terminal(
        &self,
        args: schema::CreateTerminalRequest,
    ) -> acp::Result<schema::CreateTerminalResponse> {
        match args.command.as_str() {
            "/bin/printf" => Ok(schema::CreateTerminalResponse::new("printf")),
            "/bin/sleep" => Ok(schema::CreateTerminalResponse::new("sleep")),
            _ => Err(acp::Error::invalid_params()),
        }
    }

    async fn terminal_output(
        &self,
        args: schema::TerminalOutputRequest,
    ) -> acp::Result<schema::TerminalOutputResponse> {
        if args.terminal_id.to_string() != "printf" {
            return Err(acp::Error::invalid_params());
        }
        Ok(schema::TerminalOutputResponse::new("terminal-ok", false)
            .exit_status(Some(schema::TerminalExitStatus::new().exit_code(0))))
    }

    async fn wait_for_terminal_exit(
        &self,
        args: schema::WaitForTerminalExitRequest,
    ) -> acp::Result<schema::WaitForTerminalExitResponse> {
        if args.terminal_id.to_string() != "printf" {
            return Err(acp::Error::invalid_params());
        }
        Ok(schema::WaitForTerminalExitResponse::new(
            schema::TerminalExitStatus::new().exit_code(0),
        ))
    }

    async fn kill_terminal(
        &self,
        args: schema::KillTerminalRequest,
    ) -> acp::Result<schema::KillTerminalResponse> {
        self.killed_terminals
            .lock()
            .await
            .push(args.terminal_id.to_string());
        Ok(schema::KillTerminalResponse::new())
    }

    async fn release_terminal(
        &self,
        _args: schema::ReleaseTerminalRequest,
    ) -> acp::Result<schema::ReleaseTerminalResponse> {
        Ok(schema::ReleaseTerminalResponse::new())
    }
}

#[tokio::test]
async fn mock_binary_accepts_acp_prompt_roundtrips() -> Result<()> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_acp-mock"))
        .arg("--port")
        .arg("0")
        .arg("--response-delay-ms")
        .arg("1")
        .arg("--exit-after-ms")
        .arg("500")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    let address = read_startup_url(&mut child, "acp mock listening on ").await?;

    let reply = request_reply(&address, "hello from binary test").await?;
    assert!(reply.starts_with("mock assistant:"));

    let status = timeout(Duration::from_secs(2), child.wait()).await??;
    assert!(status.success());
    Ok(())
}

#[tokio::test]
async fn mock_binary_accepts_permission_prompt_roundtrips() -> Result<()> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_acp-mock"))
        .arg("--port")
        .arg("0")
        .arg("--response-delay-ms")
        .arg("1")
        .arg("--exit-after-ms")
        .arg("500")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    let address = read_startup_url(&mut child, "acp mock listening on ").await?;

    let reply = request_reply(&address, acp_mock::MANUAL_PERMISSION_TRIGGER).await?;
    assert!(reply.starts_with("mock assistant:"));

    let status = timeout(Duration::from_secs(2), child.wait()).await??;
    assert!(status.success());
    Ok(())
}

#[tokio::test]
async fn mock_binary_accepts_runtime_tool_prompt_roundtrips() -> Result<()> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_acp-mock"))
        .arg("--port")
        .arg("0")
        .arg("--response-delay-ms")
        .arg("1")
        .arg("--exit-after-ms")
        .arg("500")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    let address = read_startup_url(&mut child, "acp mock listening on ").await?;

    let outcome =
        request_reply_with_client(&address, acp_mock::MANUAL_RUNTIME_TOOLS_TRIGGER).await?;
    assert!(outcome.reply.contains("Runtime tools verified"));
    assert!(outcome.reply.contains("binary-readme"));
    assert_eq!(
        outcome.written_file,
        Some((
            "/workspace/acp-mock-runtime-tools.txt".to_string(),
            "created by acp-mock runtime tools\n".to_string()
        ))
    );
    assert_eq!(outcome.killed_terminals, vec!["sleep".to_string()]);

    let status = timeout(Duration::from_secs(2), child.wait()).await??;
    assert!(status.success());
    Ok(())
}

async fn request_reply(address: &str, prompt: &str) -> Result<String> {
    Ok(request_reply_with_client(address, prompt).await?.reply)
}

#[derive(Debug, PartialEq, Eq)]
struct RequestReplyOutcome {
    reply: String,
    written_file: Option<(String, String)>,
    killed_terminals: Vec<String>,
}

macro_rules! test_request_handler {
    ($client:expr, $request:ty, $handler:ident) => {{
        let client = $client.clone();
        async move |args: $request, responder, cx| {
            $handler(client.clone(), args, responder, cx).await
        }
    }};
}

macro_rules! test_request_handlers {
    ($builder:expr, $client:expr) => {
        $builder
            .on_receive_request(
                test_request_handler!(
                    $client,
                    schema::RequestPermissionRequest,
                    respond_permission_request
                ),
                acp::on_receive_request!(),
            )
            .on_receive_request(
                test_request_handler!(
                    $client,
                    schema::ReadTextFileRequest,
                    respond_read_text_file_request
                ),
                acp::on_receive_request!(),
            )
            .on_receive_request(
                test_request_handler!(
                    $client,
                    schema::WriteTextFileRequest,
                    respond_write_text_file_request
                ),
                acp::on_receive_request!(),
            )
            .on_receive_request(
                test_request_handler!(
                    $client,
                    schema::CreateTerminalRequest,
                    respond_create_terminal_request
                ),
                acp::on_receive_request!(),
            )
            .on_receive_request(
                test_request_handler!(
                    $client,
                    schema::TerminalOutputRequest,
                    respond_terminal_output_request
                ),
                acp::on_receive_request!(),
            )
            .on_receive_request(
                test_request_handler!(
                    $client,
                    schema::WaitForTerminalExitRequest,
                    respond_wait_for_terminal_exit_request
                ),
                acp::on_receive_request!(),
            )
            .on_receive_request(
                test_request_handler!(
                    $client,
                    schema::KillTerminalRequest,
                    respond_kill_terminal_request
                ),
                acp::on_receive_request!(),
            )
            .on_receive_request(
                test_request_handler!(
                    $client,
                    schema::ReleaseTerminalRequest,
                    respond_release_terminal_request
                ),
                acp::on_receive_request!(),
            )
    };
}

async fn request_reply_with_client(address: &str, prompt: &str) -> Result<RequestReplyOutcome> {
    let stream = TcpStream::connect(address).await?;
    let (reader, writer) = stream.into_split();
    let client = TestClient::new();
    let outcome_client = client.clone();
    let notification_client = client.clone();
    let working_dir = std::env::current_dir()?;
    let prompt = prompt.to_string();
    let builder = acp::Client.builder().name("acp-mock-test");

    let reply = test_request_handlers!(builder, client)
        .on_receive_notification(
            async move |args: schema::SessionNotification, _cx| {
                forward_session_notification(notification_client.clone(), args).await
            },
            acp::on_receive_notification!(),
        )
        .connect_with(
            acp::ByteStreams::new(writer.compat_write(), reader.compat()),
            move |cx| run_prompt_roundtrip(cx, working_dir, prompt, client),
        )
        .await?;

    Ok(RequestReplyOutcome {
        reply,
        written_file: outcome_client.written_file().await,
        killed_terminals: outcome_client.killed_terminals().await,
    })
}

async fn respond_permission_request(
    client: TestClient,
    args: schema::RequestPermissionRequest,
    responder: acp::Responder<schema::RequestPermissionResponse>,
    connection: acp::ConnectionTo<acp::Agent>,
) -> acp::Result<()> {
    connection.spawn(async move {
        let result = client.request_permission(args).await;
        responder.respond_with_result(result)?;
        Ok(())
    })?;
    Ok(())
}

async fn forward_session_notification(
    client: TestClient,
    args: schema::SessionNotification,
) -> acp::Result<()> {
    client.session_notification(args).await
}

async fn respond_read_text_file_request(
    client: TestClient,
    args: schema::ReadTextFileRequest,
    responder: acp::Responder<schema::ReadTextFileResponse>,
    connection: acp::ConnectionTo<acp::Agent>,
) -> acp::Result<()> {
    connection.spawn(async move {
        let result = client.read_text_file(args).await;
        responder.respond_with_result(result)?;
        Ok(())
    })?;
    Ok(())
}

async fn respond_write_text_file_request(
    client: TestClient,
    args: schema::WriteTextFileRequest,
    responder: acp::Responder<schema::WriteTextFileResponse>,
    connection: acp::ConnectionTo<acp::Agent>,
) -> acp::Result<()> {
    connection.spawn(async move {
        let result = client.write_text_file(args).await;
        responder.respond_with_result(result)?;
        Ok(())
    })?;
    Ok(())
}

async fn respond_create_terminal_request(
    client: TestClient,
    args: schema::CreateTerminalRequest,
    responder: acp::Responder<schema::CreateTerminalResponse>,
    connection: acp::ConnectionTo<acp::Agent>,
) -> acp::Result<()> {
    connection.spawn(async move {
        let result = client.create_terminal(args).await;
        responder.respond_with_result(result)?;
        Ok(())
    })?;
    Ok(())
}

async fn respond_terminal_output_request(
    client: TestClient,
    args: schema::TerminalOutputRequest,
    responder: acp::Responder<schema::TerminalOutputResponse>,
    connection: acp::ConnectionTo<acp::Agent>,
) -> acp::Result<()> {
    connection.spawn(async move {
        let result = client.terminal_output(args).await;
        responder.respond_with_result(result)?;
        Ok(())
    })?;
    Ok(())
}

async fn respond_wait_for_terminal_exit_request(
    client: TestClient,
    args: schema::WaitForTerminalExitRequest,
    responder: acp::Responder<schema::WaitForTerminalExitResponse>,
    connection: acp::ConnectionTo<acp::Agent>,
) -> acp::Result<()> {
    connection.spawn(async move {
        let result = client.wait_for_terminal_exit(args).await;
        responder.respond_with_result(result)?;
        Ok(())
    })?;
    Ok(())
}

async fn respond_kill_terminal_request(
    client: TestClient,
    args: schema::KillTerminalRequest,
    responder: acp::Responder<schema::KillTerminalResponse>,
    connection: acp::ConnectionTo<acp::Agent>,
) -> acp::Result<()> {
    connection.spawn(async move {
        let result = client.kill_terminal(args).await;
        responder.respond_with_result(result)?;
        Ok(())
    })?;
    Ok(())
}

async fn respond_release_terminal_request(
    client: TestClient,
    args: schema::ReleaseTerminalRequest,
    responder: acp::Responder<schema::ReleaseTerminalResponse>,
    connection: acp::ConnectionTo<acp::Agent>,
) -> acp::Result<()> {
    connection.spawn(async move {
        let result = client.release_terminal(args).await;
        responder.respond_with_result(result)?;
        Ok(())
    })?;
    Ok(())
}

async fn run_prompt_roundtrip(
    connection: acp::ConnectionTo<acp::Agent>,
    working_dir: PathBuf,
    prompt: String,
    client: TestClient,
) -> acp::Result<String> {
    connection
        .send_request(
            schema::InitializeRequest::new(schema::ProtocolVersion::V1)
                .client_capabilities(
                    schema::ClientCapabilities::new()
                        .fs(schema::FileSystemCapabilities::new()
                            .read_text_file(true)
                            .write_text_file(true))
                        .terminal(true),
                )
                .client_info(
                    schema::Implementation::new("acp-mock-test", env!("CARGO_PKG_VERSION"))
                        .title("ACP Mock Binary Test"),
                ),
        )
        .block_task()
        .await?;
    let session = connection
        .send_request(schema::NewSessionRequest::new(working_dir))
        .block_task()
        .await?;
    connection
        .send_request(schema::PromptRequest::new(
            session.session_id,
            vec![prompt.into()],
        ))
        .block_task()
        .await?;
    Ok(client.reply_text().await)
}

fn content_text(content: schema::ContentBlock) -> String {
    match content {
        schema::ContentBlock::Text(text) => text.text,
        schema::ContentBlock::Image(_) => "<image>".to_string(),
        schema::ContentBlock::Audio(_) => "<audio>".to_string(),
        schema::ContentBlock::ResourceLink(link) => link.uri,
        schema::ContentBlock::Resource(_) => "<resource>".to_string(),
        _ => "<unsupported>".to_string(),
    }
}
