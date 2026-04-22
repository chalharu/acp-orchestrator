use std::{path::PathBuf, process::Stdio, sync::Arc, time::Duration};

use acp_app_support::read_startup_url;
use agent_client_protocol::{self as acp, schema};
use tokio::{net::TcpStream, process::Command, sync::Mutex, time::timeout};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Debug, Clone)]
struct TestClient {
    reply: Arc<Mutex<String>>,
}

impl TestClient {
    fn new() -> Self {
        Self {
            reply: Arc::new(Mutex::new(String::new())),
        }
    }

    async fn reply_text(&self) -> String {
        self.reply.lock().await.clone()
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

    let reply = request_reply(&address, "permission from binary test").await?;
    assert!(reply.starts_with("mock assistant:"));

    let status = timeout(Duration::from_secs(2), child.wait()).await??;
    assert!(status.success());
    Ok(())
}

async fn request_reply(address: &str, prompt: &str) -> Result<String> {
    let stream = TcpStream::connect(address).await?;
    let (reader, writer) = stream.into_split();
    let client = TestClient::new();
    let request_client = client.clone();
    let notification_client = client.clone();
    let working_dir = std::env::current_dir()?;
    let prompt = prompt.to_string();

    let reply = acp::Client
        .builder()
        .name("acp-mock-test")
        .on_receive_request(
            async move |args: schema::RequestPermissionRequest, responder, cx| {
                respond_permission_request(request_client.clone(), args, responder, cx).await
            },
            acp::on_receive_request!(),
        )
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

    Ok(reply)
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

async fn run_prompt_roundtrip(
    connection: acp::ConnectionTo<acp::Agent>,
    working_dir: PathBuf,
    prompt: String,
    client: TestClient,
) -> acp::Result<String> {
    connection
        .send_request(
            schema::InitializeRequest::new(schema::ProtocolVersion::V1).client_info(
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
