use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    env,
    future::Future,
    path::PathBuf,
    pin::Pin,
    rc::Rc,
    sync::Arc,
    time::Duration,
};

use agent_client_protocol::{self as acp, Agent as _};
use snafu::prelude::*;
use tokio::net::TcpStream;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

type Result<T, E = MockClientError> = std::result::Result<T, E>;
pub type ReplyFuture<'a> = Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>>;
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

pub trait ReplyProvider: Send + Sync + std::fmt::Debug {
    fn request_reply<'a>(&'a self, session_id: &'a str, prompt: &'a str) -> ReplyFuture<'a>;
}

#[derive(Debug, Clone)]
pub struct MockClient {
    mock_address: String,
    request_timeout: Duration,
    working_dir: PathBuf,
    upstream_sessions: Arc<tokio::sync::Mutex<HashMap<String, String>>>,
    session_locks: Arc<tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
}

#[derive(Debug, Snafu)]
pub enum MockClientError {
    #[snafu(display("reading the current working directory failed"))]
    ReadCurrentDirectory { source: std::io::Error },

    #[snafu(display("connecting to the mock ACP transport at {address} failed"))]
    Connect {
        source: std::io::Error,
        address: String,
    },

    #[snafu(display("initializing the mock ACP client failed"))]
    Initialize { source: acp::Error },

    #[snafu(display("creating a mock ACP session failed"))]
    CreateSession { source: acp::Error },

    #[snafu(display("loading a mock ACP session failed"))]
    LoadSession { source: acp::Error },

    #[snafu(display("sending the mock ACP prompt failed"))]
    SendPrompt { source: acp::Error },

    #[snafu(display("building the mock ACP runtime failed"))]
    BuildRuntime { source: std::io::Error },

    #[snafu(display("joining the mock ACP task failed"))]
    JoinTask { source: tokio::task::JoinError },

    #[snafu(display("the mock ACP request timed out after {timeout:?}"))]
    TimedOut { timeout: Duration },
}

impl MockClient {
    pub fn new(mock_address: String) -> Result<Self> {
        Self::with_timeout(mock_address, DEFAULT_REQUEST_TIMEOUT)
    }

    fn with_timeout(mock_address: String, request_timeout: Duration) -> Result<Self> {
        let working_dir = env::current_dir().context(ReadCurrentDirectorySnafu)?;
        Ok(Self {
            mock_address,
            request_timeout,
            working_dir,
            upstream_sessions: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            session_locks: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        })
    }

    pub async fn request_reply(&self, session_id: &str, prompt: &str) -> Result<String> {
        let backend_session_id = session_id.to_string();
        let prompt = prompt.to_string();
        let mock_address = self.mock_address.clone();
        let working_dir = self.working_dir.clone();
        let request_timeout = self.request_timeout;
        let upstream_sessions = self.upstream_sessions.clone();
        let session_lock = self.session_lock(&backend_session_id).await;
        let _serial = session_lock.lock().await;

        tokio::task::spawn_blocking(move || {
            drive_acp_roundtrip_blocking(
                mock_address,
                working_dir,
                backend_session_id,
                prompt,
                request_timeout,
                upstream_sessions,
            )
        })
        .await
        .context(JoinTaskSnafu)?
    }

    async fn session_lock(&self, backend_session_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut session_locks = self.session_locks.lock().await;
        session_locks
            .entry(backend_session_id.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }

    #[cfg(test)]
    async fn mapped_session_id(&self, backend_session_id: &str) -> Option<String> {
        self.upstream_sessions
            .lock()
            .await
            .get(backend_session_id)
            .cloned()
    }
}

impl ReplyProvider for MockClient {
    fn request_reply<'a>(&'a self, session_id: &'a str, prompt: &'a str) -> ReplyFuture<'a> {
        Box::pin(MockClient::request_reply(self, session_id, prompt))
    }
}

async fn drive_acp_roundtrip(
    mock_address: String,
    working_dir: PathBuf,
    backend_session_id: String,
    prompt: String,
    upstream_sessions: Arc<tokio::sync::Mutex<HashMap<String, String>>>,
) -> Result<String> {
    let stream = TcpStream::connect(&mock_address)
        .await
        .context(ConnectSnafu {
            address: mock_address.clone(),
        })?;
    let (reader, writer) = stream.into_split();
    let client = BackendAcpClient::new();
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
            initialize_connection(&conn).await?;
            let session_id = if let Some(session_id) = upstream_sessions
                .lock()
                .await
                .get(&backend_session_id)
                .cloned()
            {
                conn.load_session(acp::LoadSessionRequest::new(
                    session_id.clone(),
                    working_dir.clone(),
                ))
                .await
                .context(LoadSessionSnafu)?;
                session_id
            } else {
                let session = conn
                    .new_session(acp::NewSessionRequest::new(working_dir))
                    .await
                    .context(CreateSessionSnafu)?;
                let session_id = session.session_id.to_string();
                upstream_sessions
                    .lock()
                    .await
                    .insert(backend_session_id, session_id.clone());
                session_id
            };
            conn.prompt(acp::PromptRequest::new(session_id, vec![prompt.into()]))
                .await
                .context(SendPromptSnafu)?;
            client.wait_for_first_chunk().await;
            io_task.abort();
            let _ = io_task.await;
            Ok(client.reply_text())
        })
        .await
}

fn drive_acp_roundtrip_blocking(
    mock_address: String,
    working_dir: PathBuf,
    backend_session_id: String,
    prompt: String,
    request_timeout: Duration,
    upstream_sessions: Arc<tokio::sync::Mutex<HashMap<String, String>>>,
) -> Result<String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context(BuildRuntimeSnafu)?;

    runtime.block_on(async move {
        tokio::time::timeout(
            request_timeout,
            drive_acp_roundtrip(
                mock_address,
                working_dir,
                backend_session_id,
                prompt,
                upstream_sessions,
            ),
        )
        .await
        .map_err(|_| MockClientError::TimedOut {
            timeout: request_timeout,
        })?
    })
}

async fn initialize_connection(conn: &acp::ClientSideConnection) -> Result<()> {
    conn.initialize(
        acp::InitializeRequest::new(acp::ProtocolVersion::V1).client_info(
            acp::Implementation::new("acp-web-backend", env!("CARGO_PKG_VERSION"))
                .title("ACP Web Backend"),
        ),
    )
    .await
    .context(InitializeSnafu)?;
    Ok(())
}

#[derive(Debug, Clone)]
struct BackendAcpClient {
    collected: Rc<RefCell<String>>,
    saw_first_chunk: Rc<Cell<bool>>,
    first_chunk: Rc<tokio::sync::Notify>,
}

impl BackendAcpClient {
    fn new() -> Self {
        Self {
            collected: Rc::new(RefCell::new(String::new())),
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
        self.collected.borrow().clone()
    }
}

#[async_trait::async_trait(?Send)]
impl acp::Client for BackendAcpClient {
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
            self.collected
                .borrow_mut()
                .push_str(&content_text(chunk.content));
            if !self.saw_first_chunk.replace(true) {
                self.first_chunk.notify_waiters();
            }
        }
        Ok(())
    }
}

fn content_text(content: acp::ContentBlock) -> String {
    match content {
        acp::ContentBlock::Text(text) => text.text,
        acp::ContentBlock::Image(_) => "<image>".to_string(),
        acp::ContentBlock::Audio(_) => "<audio>".to_string(),
        acp::ContentBlock::ResourceLink(link) => link.uri,
        content => resource_placeholder(matches!(content, acp::ContentBlock::Resource(_))),
    }
}

fn resource_placeholder(is_resource: bool) -> String {
    ["<unsupported>", "<resource>"][usize::from(is_resource)].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use acp_app_support::wait_for_tcp_connect;
    use acp_mock::{MockConfig, spawn_with_shutdown_task};
    use agent_client_protocol::Client as _;
    use tokio::{net::TcpListener, sync::oneshot};

    #[tokio::test]
    async fn request_reply_collects_text_from_acp_mock() {
        let (mock_address, shutdown_tx) = spawn_mock_server(Duration::from_millis(1)).await;
        let client = MockClient::new(mock_address).expect("client construction should succeed");

        let reply = client
            .request_reply("s_test", "hello")
            .await
            .expect("mock ACP replies should succeed");

        assert!(reply.starts_with("mock assistant:"));

        let _ = shutdown_tx.send(());
    }

    #[tokio::test]
    async fn request_reply_reuses_upstream_sessions_for_the_same_backend_session() {
        let (mock_address, shutdown_tx) = spawn_mock_server(Duration::from_millis(1)).await;
        let client = MockClient::new(mock_address).expect("client construction should succeed");

        client
            .request_reply("s_alpha", "first prompt")
            .await
            .expect("first replies should succeed");
        client
            .request_reply("s_alpha", "second prompt")
            .await
            .expect("reused sessions should succeed");
        client
            .request_reply("s_beta", "third prompt")
            .await
            .expect("second backend sessions should succeed");

        assert_eq!(
            client.mapped_session_id("s_alpha").await.as_deref(),
            Some("mock_0")
        );
        assert_eq!(
            client.mapped_session_id("s_beta").await.as_deref(),
            Some("mock_1")
        );

        let _ = shutdown_tx.send(());
    }

    #[tokio::test]
    async fn request_reply_times_out_for_slow_mock_agents() {
        let (mock_address, shutdown_tx) = spawn_mock_server(Duration::from_millis(200)).await;
        let client = MockClient::with_timeout(mock_address, Duration::from_millis(20))
            .expect("client construction should succeed");

        let error = client
            .request_reply("s_test", "hello")
            .await
            .expect_err("stalled responses should time out");

        assert!(matches!(error, MockClientError::TimedOut { .. }));

        let _ = shutdown_tx.send(());
    }

    #[tokio::test]
    async fn request_reply_reports_connect_failures() {
        let client = MockClient::with_timeout("127.0.0.1:9".to_string(), Duration::from_millis(20))
            .expect("client construction should succeed");

        let error = client
            .request_reply("s_test", "hello")
            .await
            .expect_err("unreachable mock transports should fail");

        assert!(matches!(error, MockClientError::Connect { .. }));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn backend_acp_client_waits_until_the_first_chunk_arrives() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let client = BackendAcpClient::new();
                let waiter = tokio::task::spawn_local({
                    let client = client.clone();
                    async move {
                        client.wait_for_first_chunk().await;
                    }
                });

                tokio::task::yield_now().await;
                client
                    .session_notification(acp::SessionNotification::new(
                        "mock_0",
                        acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(
                            "first chunk".into(),
                        )),
                    ))
                    .await
                    .expect("session updates should succeed");

                waiter
                    .await
                    .expect("waiting for the first chunk should complete");
                assert_eq!(client.reply_text(), "first chunk");
            })
            .await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn backend_acp_client_rejects_permission_requests() {
        let client = BackendAcpClient::new();

        assert!(
            client
                .request_permission(acp::RequestPermissionRequest::new(
                    "mock_0",
                    acp::ToolCallUpdate::new(
                        "tool_0",
                        acp::ToolCallUpdateFields::new().title("permission prompt"),
                    ),
                    vec![acp::PermissionOption::new(
                        "allow_once",
                        "Allow once",
                        acp::PermissionOptionKind::AllowOnce,
                    )],
                ))
                .await
                .is_err()
        );
    }

    #[test]
    fn content_text_formats_embedded_resources() {
        let resource = acp::ContentBlock::Resource(acp::EmbeddedResource::new(
            acp::EmbeddedResourceResource::TextResourceContents(acp::TextResourceContents::new(
                "hello",
                "file:///embedded.md",
            )),
        ));

        assert_eq!(content_text(resource), "<resource>");
    }

    #[test]
    fn content_text_formats_non_text_prompt_blocks() {
        assert_eq!(
            content_text(acp::ContentBlock::Image(acp::ImageContent::new(
                "aGVsbG8=",
                "image/png",
            ))),
            "<image>"
        );
        assert_eq!(
            content_text(acp::ContentBlock::Audio(acp::AudioContent::new(
                "aGVsbG8=",
                "audio/wav",
            ))),
            "<audio>"
        );
        assert_eq!(
            content_text(acp::ContentBlock::ResourceLink(acp::ResourceLink::new(
                "guide",
                "file:///guide.md",
            ))),
            "file:///guide.md"
        );
    }

    async fn spawn_mock_server(delay: Duration) -> (String, oneshot::Sender<()>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let address = listener
            .local_addr()
            .expect("listener should expose its address");
        let (shutdown_tx, shutdown_rx) = oneshot::channel();

        spawn_with_shutdown_task(
            listener,
            MockConfig {
                response_delay: delay,
            },
            async move {
                let _ = shutdown_rx.await;
            },
        );

        wait_for_tcp_connect(&address.to_string(), 20, Duration::from_millis(10))
            .await
            .expect("mock server should accept TCP connections");

        (address.to_string(), shutdown_tx)
    }
}
