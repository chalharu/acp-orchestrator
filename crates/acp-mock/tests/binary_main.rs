use std::{
    cell::{Cell, RefCell},
    process::Stdio,
    rc::Rc,
    time::Duration,
};

use acp_app_support::read_startup_url;
use agent_client_protocol::{self as acp, Agent as _};
use tokio::{net::TcpStream, process::Command, time::timeout};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Debug, Clone)]
struct TestClient {
    reply: Rc<RefCell<String>>,
    saw_first_chunk: Rc<Cell<bool>>,
    first_chunk: Rc<tokio::sync::Notify>,
}

impl TestClient {
    fn new() -> Self {
        Self {
            reply: Rc::new(RefCell::new(String::new())),
            saw_first_chunk: Rc::new(Cell::new(false)),
            first_chunk: Rc::new(tokio::sync::Notify::new()),
        }
    }

    async fn wait_for_first_chunk(&self) {
        if !self.saw_first_chunk.get() {
            self.first_chunk.notified().await;
        }
    }

    fn reply_text(&self) -> String {
        self.reply.borrow().clone()
    }
}

#[async_trait::async_trait(?Send)]
impl acp::Client for TestClient {
    async fn request_permission(
        &self,
        _args: acp::RequestPermissionRequest,
    ) -> acp::Result<acp::RequestPermissionResponse> {
        Err(acp::Error::method_not_found())
    }

    async fn session_notification(
        &self,
        args: acp::SessionNotification,
    ) -> acp::Result<(), acp::Error> {
        if let acp::SessionUpdate::AgentMessageChunk(chunk) = args.update {
            self.reply
                .borrow_mut()
                .push_str(&content_text(chunk.content));
            if !self.saw_first_chunk.replace(true) {
                self.first_chunk.notify_waiters();
            }
        }
        Ok(())
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

async fn request_reply(address: &str, prompt: &str) -> Result<String> {
    let stream = TcpStream::connect(address).await?;
    let (reader, writer) = stream.into_split();
    let client = TestClient::new();
    let working_dir = std::env::current_dir()?;
    let local_set = tokio::task::LocalSet::new();

    local_set
        .run_until(async move {
            let (conn, handle_io) = acp::ClientSideConnection::new(
                client.clone(),
                writer.compat_write(),
                reader.compat(),
                |future| {
                    tokio::task::spawn_local(future);
                },
            );
            let io_task = tokio::task::spawn_local(handle_io);
            conn.initialize(
                acp::InitializeRequest::new(acp::ProtocolVersion::V1).client_info(
                    acp::Implementation::new("acp-mock-test", env!("CARGO_PKG_VERSION"))
                        .title("ACP Mock Binary Test"),
                ),
            )
            .await?;
            let session = conn
                .new_session(acp::NewSessionRequest::new(working_dir))
                .await?;
            conn.prompt(acp::PromptRequest::new(
                session.session_id,
                vec![prompt.to_string().into()],
            ))
            .await?;
            client.wait_for_first_chunk().await;
            io_task.abort();
            let _ = io_task.await;
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(client.reply_text())
        })
        .await
}

fn content_text(content: acp::ContentBlock) -> String {
    match content {
        acp::ContentBlock::Text(text) => text.text,
        acp::ContentBlock::Image(_) => "<image>".to_string(),
        acp::ContentBlock::Audio(_) => "<audio>".to_string(),
        acp::ContentBlock::ResourceLink(link) => link.uri,
        acp::ContentBlock::Resource(_) => "<resource>".to_string(),
        _ => "<unsupported>".to_string(),
    }
}
