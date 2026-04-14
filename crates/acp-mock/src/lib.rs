pub mod runtime;

use std::{cell::Cell, future::Future, rc::Rc, time::Duration};

use agent_client_protocol::{self as acp, Client as _};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::{mpsc, oneshot},
    time::sleep,
};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use tracing::{error, info};

pub use runtime::{MockAppError, run_with_args};

#[derive(Debug, Clone)]
pub struct MockConfig {
    pub response_delay: Duration,
}

impl Default for MockConfig {
    fn default() -> Self {
        Self {
            response_delay: Duration::from_millis(120),
        }
    }
}

#[derive(Debug)]
struct MockServerState {
    config: MockConfig,
    next_session_id: Cell<u64>,
}

impl MockServerState {
    fn new(config: MockConfig) -> Self {
        Self {
            config,
            next_session_id: Cell::new(0),
        }
    }

    fn next_session_id(&self) -> String {
        let next = self.next_session_id.get();
        self.next_session_id.set(next + 1);
        format!("mock_{next}")
    }
}

type QueuedSessionNotification = (acp::SessionNotification, oneshot::Sender<()>);

#[async_trait::async_trait(?Send)]
trait SessionUpdateNotifier {
    async fn send_session_update(
        &self,
        notification: acp::SessionNotification,
    ) -> Result<(), acp::Error>;
}

struct MockAgent {
    state: Rc<MockServerState>,
    session_update_tx: mpsc::UnboundedSender<QueuedSessionNotification>,
}

impl MockAgent {
    fn new(
        state: Rc<MockServerState>,
        session_update_tx: mpsc::UnboundedSender<QueuedSessionNotification>,
    ) -> Self {
        Self {
            state,
            session_update_tx,
        }
    }

    async fn send_reply(&self, session_id: String, text: String) -> Result<(), acp::Error> {
        let (ack_tx, ack_rx) = oneshot::channel();
        self.session_update_tx
            .send((
                acp::SessionNotification::new(
                    session_id,
                    acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(text.into())),
                ),
                ack_tx,
            ))
            .map_err(|_| acp::Error::internal_error())?;
        ack_rx.await.map_err(|_| acp::Error::internal_error())
    }
}

#[async_trait::async_trait(?Send)]
impl acp::Agent for MockAgent {
    async fn initialize(
        &self,
        _arguments: acp::InitializeRequest,
    ) -> Result<acp::InitializeResponse, acp::Error> {
        Ok(
            acp::InitializeResponse::new(acp::ProtocolVersion::V1).agent_info(
                acp::Implementation::new("acp-mock", env!("CARGO_PKG_VERSION")).title("ACP Mock"),
            ),
        )
    }

    async fn authenticate(
        &self,
        _arguments: acp::AuthenticateRequest,
    ) -> Result<acp::AuthenticateResponse, acp::Error> {
        Ok(acp::AuthenticateResponse::default())
    }

    async fn new_session(
        &self,
        _arguments: acp::NewSessionRequest,
    ) -> Result<acp::NewSessionResponse, acp::Error> {
        Ok(acp::NewSessionResponse::new(self.state.next_session_id()))
    }

    async fn load_session(
        &self,
        _arguments: acp::LoadSessionRequest,
    ) -> Result<acp::LoadSessionResponse, acp::Error> {
        Ok(acp::LoadSessionResponse::new())
    }

    async fn prompt(
        &self,
        arguments: acp::PromptRequest,
    ) -> Result<acp::PromptResponse, acp::Error> {
        let prompt = prompt_text(&arguments.prompt);
        sleep(self.state.config.response_delay).await;
        self.send_reply(arguments.session_id.to_string(), reply_for(&prompt))
            .await?;
        Ok(acp::PromptResponse::new(acp::StopReason::EndTurn))
    }

    async fn cancel(&self, _args: acp::CancelNotification) -> Result<(), acp::Error> {
        Ok(())
    }

    async fn set_session_mode(
        &self,
        _args: acp::SetSessionModeRequest,
    ) -> Result<acp::SetSessionModeResponse, acp::Error> {
        Ok(acp::SetSessionModeResponse::default())
    }
}

#[async_trait::async_trait(?Send)]
impl SessionUpdateNotifier for acp::AgentSideConnection {
    async fn send_session_update(
        &self,
        notification: acp::SessionNotification,
    ) -> Result<(), acp::Error> {
        self.session_notification(notification).await
    }
}

pub async fn serve_with_shutdown<F>(
    listener: TcpListener,
    config: MockConfig,
    shutdown: F,
) -> std::io::Result<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    let address = listener.local_addr()?;
    info!("starting acp mock on {address}");

    let state = Rc::new(MockServerState::new(config));
    let local_set = tokio::task::LocalSet::new();
    local_set
        .run_until(async move {
            tokio::pin!(shutdown);

            loop {
                tokio::select! {
                    _ = &mut shutdown => return Ok(()),
                    accepted = listener.accept() => {
                        let (stream, _) = accepted?;
                        let state = state.clone();
                        tokio::task::spawn_local(async move {
                            log_connection_result(handle_connection(stream, state).await);
                        });
                    }
                }
            }
        })
        .await
}

pub fn spawn_with_shutdown_task<F>(
    listener: TcpListener,
    config: MockConfig,
    shutdown: F,
) -> tokio::task::JoinHandle<std::io::Result<()>>
where
    F: Future<Output = ()> + Send + 'static,
{
    tokio::task::spawn_blocking(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        runtime.block_on(async move { serve_with_shutdown(listener, config, shutdown).await })
    })
}

async fn handle_connection(
    stream: TcpStream,
    state: Rc<MockServerState>,
) -> Result<(), acp::Error> {
    let (reader, writer) = stream.into_split();
    let (session_update_tx, session_update_rx) = mpsc::unbounded_channel();
    let (conn, handle_io) = acp::AgentSideConnection::new(
        MockAgent::new(state, session_update_tx),
        writer.compat_write(),
        reader.compat(),
        |future| {
            tokio::task::spawn_local(future);
        },
    );

    tokio::task::spawn_local(async move {
        drain_session_updates(&conn, session_update_rx).await;
    });

    handle_io.await
}

fn log_connection_result(result: Result<(), acp::Error>) {
    if let Err(error) = result {
        error!("mock ACP connection failed: {error}");
    }
}

fn finalize_session_update(result: Result<(), acp::Error>, ack_tx: oneshot::Sender<()>) -> bool {
    if let Err(error) = result {
        error!("sending mock ACP session update failed: {error}");
        return false;
    }

    let _ = ack_tx.send(());
    true
}

async fn drain_session_updates<N: SessionUpdateNotifier>(
    notifier: &N,
    mut session_update_rx: mpsc::UnboundedReceiver<QueuedSessionNotification>,
) {
    while let Some((notification, ack_tx)) = session_update_rx.recv().await {
        let result = notifier.send_session_update(notification).await;
        if !finalize_session_update(result, ack_tx) {
            return;
        }
    }
}

fn prompt_text(prompt: &[acp::ContentBlock]) -> String {
    prompt
        .iter()
        .map(content_text)
        .collect::<Vec<_>>()
        .join(" ")
}

fn content_text(content: &acp::ContentBlock) -> String {
    match content {
        acp::ContentBlock::Text(text) => text.text.clone(),
        acp::ContentBlock::Image(_) => "<image>".to_string(),
        acp::ContentBlock::Audio(_) => "<audio>".to_string(),
        acp::ContentBlock::ResourceLink(link) => link.uri.clone(),
        content => resource_placeholder(matches!(content, acp::ContentBlock::Resource(_))),
    }
}

fn resource_placeholder(is_resource: bool) -> String {
    ["<unsupported>", "<resource>"][usize::from(is_resource)].to_string()
}

pub fn reply_for(prompt: &str) -> String {
    let compact = prompt.split_whitespace().collect::<Vec<_>>().join(" ");

    format!(
        "mock assistant: I received `{}`. The backend-to-mock ACP round-trip succeeded.",
        truncate(&compact, 120)
    )
}

fn truncate(value: &str, max_len: usize) -> String {
    let mut chars = value.chars();
    let truncated = chars.by_ref().take(max_len).collect::<String>();

    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol::Agent as _;
    use std::rc::Rc;

    struct StubSessionUpdateNotifier {
        should_fail: bool,
        call_count: Rc<Cell<usize>>,
    }

    #[async_trait::async_trait(?Send)]
    impl SessionUpdateNotifier for StubSessionUpdateNotifier {
        async fn send_session_update(
            &self,
            _notification: acp::SessionNotification,
        ) -> Result<(), acp::Error> {
            self.call_count.set(self.call_count.get() + 1);
            if self.should_fail {
                Err(acp::Error::internal_error())
            } else {
                Ok(())
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn mock_agent_supports_control_plane_requests() {
        let state = Rc::new(MockServerState::new(MockConfig::default()));
        let (session_update_tx, _session_update_rx) = mpsc::unbounded_channel();
        let agent = MockAgent::new(state, session_update_tx);

        let authenticate = agent
            .authenticate(acp::AuthenticateRequest::new("local"))
            .await
            .expect("authenticate requests should succeed");
        assert_eq!(authenticate, acp::AuthenticateResponse::default());

        let session = agent
            .new_session(acp::NewSessionRequest::new("/tmp"))
            .await
            .expect("new sessions should succeed");
        assert_eq!(session.session_id.to_string(), "mock_0");

        let loaded = agent
            .load_session(acp::LoadSessionRequest::new(
                session.session_id.clone(),
                "/tmp",
            ))
            .await
            .expect("load session requests should succeed");
        assert_eq!(loaded, acp::LoadSessionResponse::new());

        agent
            .cancel(acp::CancelNotification::new(session.session_id.clone()))
            .await
            .expect("cancel notifications should succeed");

        let mode = agent
            .set_session_mode(acp::SetSessionModeRequest::new(
                session.session_id,
                "default",
            ))
            .await
            .expect("set session mode requests should succeed");
        assert_eq!(mode, acp::SetSessionModeResponse::default());
    }

    #[test]
    fn default_config_uses_the_expected_delay() {
        assert_eq!(
            MockConfig::default().response_delay,
            Duration::from_millis(120)
        );
    }

    #[test]
    fn prompt_text_formats_binary_placeholders_and_resource_links() {
        let prompt = vec![
            acp::ContentBlock::Image(acp::ImageContent::new("aGVsbG8=", "image/png")),
            acp::ContentBlock::Audio(acp::AudioContent::new("aGVsbG8=", "audio/wav")),
            acp::ContentBlock::ResourceLink(acp::ResourceLink::new("guide", "file:///guide.md")),
            acp::ContentBlock::Resource(acp::EmbeddedResource::new(
                acp::EmbeddedResourceResource::TextResourceContents(
                    acp::TextResourceContents::new("hello", "file:///embedded.md"),
                ),
            )),
        ];

        assert_eq!(
            prompt_text(&prompt),
            "<image> <audio> file:///guide.md <resource>"
        );
    }

    #[test]
    fn long_prompts_are_truncated_in_mock_replies() {
        let prompt = "word ".repeat(80);
        let reply = reply_for(&prompt);

        assert!(reply.contains("...`"));
        assert!(reply.starts_with("mock assistant: I received `"));
        assert!(reply.contains("ACP round-trip succeeded"));
    }

    #[test]
    fn logging_connection_errors_does_not_panic() {
        log_connection_result(Err(acp::Error::internal_error()));
    }

    #[test]
    fn finalizing_session_updates_stops_after_errors() {
        let (ack_tx, ack_rx) = oneshot::channel();

        assert!(!finalize_session_update(
            Err(acp::Error::internal_error()),
            ack_tx
        ));
        assert!(ack_rx.blocking_recv().is_err());
    }

    #[test]
    fn finalizing_session_updates_acknowledges_successful_notifications() {
        let (ack_tx, ack_rx) = oneshot::channel();

        assert!(finalize_session_update(Ok(()), ack_tx));
        assert!(ack_rx.blocking_recv().is_ok());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn draining_session_updates_returns_after_notification_errors() {
        let call_count = Rc::new(Cell::new(0));
        let notifier = StubSessionUpdateNotifier {
            should_fail: true,
            call_count: call_count.clone(),
        };
        let (session_update_tx, session_update_rx) = mpsc::unbounded_channel();
        let (ack_tx, ack_rx) = oneshot::channel();
        session_update_tx
            .send((
                acp::SessionNotification::new(
                    "mock_0",
                    acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new("hello".into())),
                ),
                ack_tx,
            ))
            .expect("session update should queue");
        drop(session_update_tx);

        drain_session_updates(&notifier, session_update_rx).await;

        assert_eq!(call_count.get(), 1);
        assert!(ack_rx.await.is_err());
    }
}
