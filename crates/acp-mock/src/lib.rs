pub mod runtime;

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    future::Future,
    rc::Rc,
    time::Duration,
};

use agent_client_protocol::{self as acp, Client as _};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::{mpsc, oneshot, watch},
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
    next_tool_call_id: Cell<u64>,
    sessions: RefCell<HashMap<String, Rc<MockSessionState>>>,
}

impl MockServerState {
    fn new(config: MockConfig) -> Self {
        Self {
            config,
            next_session_id: Cell::new(0),
            next_tool_call_id: Cell::new(0),
            sessions: RefCell::new(HashMap::new()),
        }
    }

    fn next_session_id(&self) -> String {
        let next = self.next_session_id.get();
        self.next_session_id.set(next + 1);
        let session_id = format!("mock_{next}");
        self.sessions
            .borrow_mut()
            .entry(session_id.clone())
            .or_insert_with(|| Rc::new(MockSessionState::new()));
        session_id
    }

    fn next_tool_call_id(&self) -> String {
        let next = self.next_tool_call_id.get();
        self.next_tool_call_id.set(next + 1);
        format!("tool_{next}")
    }

    fn session_state(&self, session_id: &str) -> Rc<MockSessionState> {
        self.sessions
            .borrow_mut()
            .entry(session_id.to_string())
            .or_insert_with(|| Rc::new(MockSessionState::new()))
            .clone()
    }
}

type QueuedSessionNotification = (acp::SessionNotification, oneshot::Sender<()>);
type QueuedPermissionRequest = (
    acp::RequestPermissionRequest,
    oneshot::Sender<Result<acp::RequestPermissionResponse, acp::Error>>,
);

#[derive(Debug)]
struct MockSessionState {
    cancel_generation: Cell<u64>,
    cancel_tx: watch::Sender<u64>,
}

impl MockSessionState {
    fn new() -> Self {
        let (cancel_tx, _) = watch::channel(0);
        Self {
            cancel_generation: Cell::new(0),
            cancel_tx,
        }
    }

    fn subscribe_cancel(&self) -> (watch::Receiver<u64>, u64) {
        let cancel_rx = self.cancel_tx.subscribe();
        let generation = *cancel_rx.borrow();
        (cancel_rx, generation)
    }

    fn cancel(&self) {
        let next_generation = self.cancel_generation.get() + 1;
        self.cancel_generation.set(next_generation);
        let _ = self.cancel_tx.send(next_generation);
    }
}

#[async_trait::async_trait(?Send)]
trait SessionUpdateNotifier {
    async fn send_session_update(
        &self,
        notification: acp::SessionNotification,
    ) -> Result<(), acp::Error>;
}

#[async_trait::async_trait(?Send)]
trait PermissionRequester {
    async fn request_permission(
        &self,
        request: acp::RequestPermissionRequest,
    ) -> Result<acp::RequestPermissionResponse, acp::Error>;
}

struct MockAgent {
    state: Rc<MockServerState>,
    session_update_tx: mpsc::UnboundedSender<QueuedSessionNotification>,
    permission_request_tx: mpsc::UnboundedSender<QueuedPermissionRequest>,
}

impl MockAgent {
    fn new(
        state: Rc<MockServerState>,
        session_update_tx: mpsc::UnboundedSender<QueuedSessionNotification>,
        permission_request_tx: mpsc::UnboundedSender<QueuedPermissionRequest>,
    ) -> Self {
        Self {
            state,
            session_update_tx,
            permission_request_tx,
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

    async fn request_permission(
        &self,
        session_id: String,
    ) -> Result<acp::RequestPermissionResponse, acp::Error> {
        let (ack_tx, ack_rx) = oneshot::channel();
        self.permission_request_tx
            .send((
                acp::RequestPermissionRequest::new(
                    session_id,
                    acp::ToolCallUpdate::new(
                        self.state.next_tool_call_id(),
                        acp::ToolCallUpdateFields::new().title("read_text_file README.md"),
                    ),
                    vec![
                        acp::PermissionOption::new(
                            "allow_once",
                            "Allow once",
                            acp::PermissionOptionKind::AllowOnce,
                        ),
                        acp::PermissionOption::new(
                            "reject_once",
                            "Reject once",
                            acp::PermissionOptionKind::RejectOnce,
                        ),
                    ],
                ),
                ack_tx,
            ))
            .map_err(|_| acp::Error::internal_error())?;
        ack_rx.await.map_err(|_| acp::Error::internal_error())?
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
        arguments: acp::LoadSessionRequest,
    ) -> Result<acp::LoadSessionResponse, acp::Error> {
        let _ = self.state.session_state(&arguments.session_id.to_string());
        Ok(acp::LoadSessionResponse::new())
    }

    async fn prompt(
        &self,
        arguments: acp::PromptRequest,
    ) -> Result<acp::PromptResponse, acp::Error> {
        let prompt = prompt_text(&arguments.prompt);
        let session_id = arguments.session_id.to_string();
        let session_state = self.state.session_state(&session_id);
        let (mut cancel_rx, cancel_generation) = session_state.subscribe_cancel();

        if prompt_requires_permission(&prompt) {
            match self.request_permission(session_id.clone()).await?.outcome {
                acp::RequestPermissionOutcome::Cancelled => {
                    return Ok(acp::PromptResponse::new(acp::StopReason::Cancelled));
                }
                acp::RequestPermissionOutcome::Selected(selected)
                    if selected.option_id.to_string() == "reject_once" =>
                {
                    return Ok(acp::PromptResponse::new(acp::StopReason::EndTurn));
                }
                acp::RequestPermissionOutcome::Selected(_) => {}
                _ => return Ok(acp::PromptResponse::new(acp::StopReason::Cancelled)),
            }
        }

        if wait_for_cancel(
            &mut cancel_rx,
            cancel_generation,
            self.state.config.response_delay,
        )
        .await
        {
            return Ok(acp::PromptResponse::new(acp::StopReason::Cancelled));
        }

        self.send_reply(session_id, reply_for(&prompt)).await?;
        Ok(acp::PromptResponse::new(acp::StopReason::EndTurn))
    }

    async fn cancel(&self, args: acp::CancelNotification) -> Result<(), acp::Error> {
        self.state
            .session_state(&args.session_id.to_string())
            .cancel();
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

#[async_trait::async_trait(?Send)]
impl PermissionRequester for acp::AgentSideConnection {
    async fn request_permission(
        &self,
        request: acp::RequestPermissionRequest,
    ) -> Result<acp::RequestPermissionResponse, acp::Error> {
        acp::Client::request_permission(self, request).await
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
    let (permission_request_tx, permission_request_rx) = mpsc::unbounded_channel();
    let (conn, handle_io) = acp::AgentSideConnection::new(
        MockAgent::new(state, session_update_tx, permission_request_tx),
        writer.compat_write(),
        reader.compat(),
        |future| {
            tokio::task::spawn_local(future);
        },
    );
    let conn = Rc::new(conn);
    let session_updates_conn = conn.clone();
    let permission_requests_conn = conn.clone();

    tokio::task::spawn_local(async move {
        drain_session_updates(&*session_updates_conn, session_update_rx).await;
    });
    tokio::task::spawn_local(async move {
        drain_permission_requests(&*permission_requests_conn, permission_request_rx).await;
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

fn finalize_permission_request(
    result: Result<acp::RequestPermissionResponse, acp::Error>,
    ack_tx: oneshot::Sender<Result<acp::RequestPermissionResponse, acp::Error>>,
) -> bool {
    let should_continue = result.is_ok();
    if let Err(error) = ack_tx.send(result) {
        error!("sending mock ACP permission outcome failed: {error:?}");
        return false;
    }
    should_continue
}

async fn drain_permission_requests<N: PermissionRequester>(
    requester: &N,
    mut permission_request_rx: mpsc::UnboundedReceiver<QueuedPermissionRequest>,
) {
    while let Some((request, ack_tx)) = permission_request_rx.recv().await {
        let result = requester.request_permission(request).await;
        if !finalize_permission_request(result, ack_tx) {
            break;
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

fn prompt_requires_permission(prompt: &str) -> bool {
    prompt.to_ascii_lowercase().contains("permission")
}

async fn wait_for_cancel(
    cancel_rx: &mut watch::Receiver<u64>,
    start_generation: u64,
    response_delay: Duration,
) -> bool {
    if *cancel_rx.borrow() != start_generation {
        return true;
    }

    tokio::select! {
        _ = sleep(response_delay) => false,
        changed = cancel_rx.changed() => changed.is_ok() && *cancel_rx.borrow() != start_generation,
    }
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

    struct StubPermissionRequester {
        call_count: Rc<Cell<usize>>,
    }

    #[async_trait::async_trait(?Send)]
    impl PermissionRequester for StubPermissionRequester {
        async fn request_permission(
            &self,
            _request: acp::RequestPermissionRequest,
        ) -> Result<acp::RequestPermissionResponse, acp::Error> {
            self.call_count.set(self.call_count.get() + 1);
            Ok(acp::RequestPermissionResponse::new(
                acp::RequestPermissionOutcome::Cancelled,
            ))
        }
    }

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
        let (permission_request_tx, _permission_request_rx) = mpsc::unbounded_channel();
        let agent = MockAgent::new(state, session_update_tx, permission_request_tx);

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
    fn finalizing_permission_requests_acknowledges_successful_notifications() {
        let (ack_tx, ack_rx) = oneshot::channel();

        assert!(finalize_permission_request(
            Ok(acp::RequestPermissionResponse::new(
                acp::RequestPermissionOutcome::Cancelled,
            )),
            ack_tx,
        ));
        let response = ack_rx
            .blocking_recv()
            .expect("permission responses should be forwarded")
            .expect("forwarded responses should stay successful");
        assert!(matches!(
            response.outcome,
            acp::RequestPermissionOutcome::Cancelled
        ));
    }

    #[test]
    fn finalizing_permission_requests_stop_after_receiver_drop() {
        let (ack_tx, ack_rx) = oneshot::channel();
        drop(ack_rx);

        assert!(!finalize_permission_request(
            Ok(acp::RequestPermissionResponse::new(
                acp::RequestPermissionOutcome::Cancelled,
            )),
            ack_tx,
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn draining_permission_requests_returns_after_delivery_errors() {
        let call_count = Rc::new(Cell::new(0));
        let requester = StubPermissionRequester {
            call_count: call_count.clone(),
        };
        let (permission_request_tx, permission_request_rx) = mpsc::unbounded_channel();
        let (first_ack_tx, first_ack_rx) = oneshot::channel();
        let (second_ack_tx, second_ack_rx) = oneshot::channel();
        drop(first_ack_rx);
        permission_request_tx
            .send((
                acp::RequestPermissionRequest::new(
                    "mock_0",
                    acp::ToolCallUpdate::new("tool_0", acp::ToolCallUpdateFields::new()),
                    vec![],
                ),
                first_ack_tx,
            ))
            .expect("first permission request should queue");
        permission_request_tx
            .send((
                acp::RequestPermissionRequest::new(
                    "mock_0",
                    acp::ToolCallUpdate::new("tool_1", acp::ToolCallUpdateFields::new()),
                    vec![],
                ),
                second_ack_tx,
            ))
            .expect("second permission request should queue");
        drop(permission_request_tx);

        drain_permission_requests(&requester, permission_request_rx).await;

        assert_eq!(call_count.get(), 1);
        assert!(second_ack_rx.await.is_err());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn mock_agent_permission_requests_include_expected_options() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let state = Rc::new(MockServerState::new(MockConfig::default()));
                let (session_update_tx, _session_update_rx) = mpsc::unbounded_channel();
                let (permission_request_tx, mut permission_request_rx) = mpsc::unbounded_channel();
                let agent = MockAgent::new(state, session_update_tx, permission_request_tx);

                let request_task = tokio::task::spawn_local(async move {
                    agent
                        .request_permission("mock_0".to_string())
                        .await
                        .expect("permission requests should resolve")
                });

                let (request, ack_tx) = permission_request_rx
                    .recv()
                    .await
                    .expect("permission requests should be queued");
                assert_eq!(request.session_id.to_string(), "mock_0");
                assert_eq!(request.tool_call.tool_call_id.to_string(), "tool_0");
                assert_eq!(
                    request.tool_call.fields.title.as_deref(),
                    Some("read_text_file README.md")
                );
                assert_eq!(request.options.len(), 2);
                assert_eq!(request.options[0].option_id.to_string(), "allow_once");
                assert_eq!(request.options[1].option_id.to_string(), "reject_once");

                ack_tx
                    .send(Ok(acp::RequestPermissionResponse::new(
                        acp::RequestPermissionOutcome::Selected(
                            acp::SelectedPermissionOutcome::new("allow_once"),
                        ),
                    )))
                    .expect("permission request outcomes should be delivered");

                let response = request_task
                    .await
                    .expect("permission request task should finish");
                assert!(matches!(
                    response.outcome,
                    acp::RequestPermissionOutcome::Selected(selected)
                        if selected.option_id.to_string() == "allow_once"
                ));
            })
            .await;
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

    #[tokio::test(flavor = "current_thread")]
    async fn wait_for_cancel_returns_true_when_the_generation_has_advanced() {
        let session = MockSessionState::new();
        let (mut cancel_rx, generation) = session.subscribe_cancel();
        session.cancel();

        assert!(wait_for_cancel(&mut cancel_rx, generation, Duration::from_secs(1)).await);
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
