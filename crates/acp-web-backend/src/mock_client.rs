use std::{
    collections::HashMap,
    env,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::Arc,
    time::{Duration, Instant},
};

use agent_client_protocol::{self as acp, ConnectTo, schema};
use futures_util::{
    Sink, Stream,
    future::{self, BoxFuture, Either},
    io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader},
    sink::unfold,
};
use snafu::prelude::*;
use tokio::net::{
    TcpStream,
    tcp::{OwnedReadHalf, OwnedWriteHalf},
};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

mod backend_client;

#[cfg(test)]
mod tests;

use crate::{agent_runtime::AgentLaunchMetadata, sessions::TurnHandle};
use backend_client::BackendAcpClient;

type Result<T, E = MockClientError> = std::result::Result<T, E>;
pub type ReplyFuture<'a> = Pin<Box<dyn Future<Output = Result<ReplyResult>> + Send + 'a>>;
pub type PrimeSessionFuture<'a> = Pin<Box<dyn Future<Output = Result<Option<String>>> + Send + 'a>>;
pub type BindSessionFuture<'a> =
    Pin<Box<dyn Future<Output = std::result::Result<(), String>> + Send + 'a>>;
pub type BindLaunchMetadataFuture<'a> =
    Pin<Box<dyn Future<Output = std::result::Result<(), String>> + Send + 'a>>;
type DynRd = Box<dyn AsyncRead + Send + Unpin>;
type DynWr = Box<dyn AsyncWrite + Send + Unpin>;
type LineSink = Pin<Box<dyn Sink<String, Error = std::io::Error> + Send>>;
type LineStream = Pin<Box<dyn Stream<Item = std::io::Result<String>> + Send>>;
type OperationFuture<T> = Pin<Box<dyn Future<Output = Result<T>> + Send>>;
type ConnectedOperation<T> = Box<
    dyn FnOnce(
            acp::ConnectionTo<acp::Agent>,
            PathBuf,
            String,
            BackendAcpClient,
            UpstreamSessions,
            AgentConnectionCapabilities,
        ) -> OperationFuture<T>
        + Send,
>;
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_RETRY_MAX: Duration = Duration::from_secs(10);
const CONNECT_RETRY_DELAY: Duration = Duration::from_millis(25);
type UpstreamSessions = Arc<tokio::sync::Mutex<HashMap<String, String>>>;
type SessionWorkingDirs = Arc<tokio::sync::Mutex<HashMap<String, PathBuf>>>;
type SessionAcpAddresses = Arc<tokio::sync::Mutex<HashMap<String, String>>>;
type SessionLock = Arc<tokio::sync::Mutex<()>>;
type SessionLocks = Arc<tokio::sync::Mutex<HashMap<String, SessionLock>>>;

struct ConnectedMainState<T> {
    working_dir: PathBuf,
    backend_session_id: String,
    client: BackendAcpClient,
    upstream_sessions: UpstreamSessions,
    operation: ConnectedOperation<T>,
}

#[derive(Debug, Clone, Copy)]
struct AgentConnectionCapabilities {
    load_session: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplyResult {
    Reply(String),
    Status(String),
    NoOutput,
}

pub trait ReplyProvider: Send + Sync + std::fmt::Debug {
    fn request_reply<'a>(&'a self, turn: TurnHandle) -> ReplyFuture<'a>;

    fn bind_session<'a>(
        &'a self,
        _session_id: &'a str,
        _working_dir: PathBuf,
    ) -> BindSessionFuture<'a> {
        Box::pin(async { Ok(()) })
    }

    fn bind_session_launch_metadata<'a>(
        &'a self,
        _session_id: &'a str,
        _metadata: AgentLaunchMetadata,
    ) -> BindLaunchMetadataFuture<'a> {
        Box::pin(async { Ok(()) })
    }

    fn prime_session<'a>(&'a self, _session_id: &'a str) -> PrimeSessionFuture<'a> {
        Box::pin(async { Ok(None) })
    }

    fn forget_session(&self, _session_id: &str) {}
}

#[derive(Debug, Clone)]
pub struct MockClient {
    mock_address: String,
    request_timeout: Duration,
    default_working_dir: PathBuf,
    session_working_dirs: SessionWorkingDirs,
    session_acp_addresses: SessionAcpAddresses,
    upstream_sessions: UpstreamSessions,
    session_locks: SessionLocks,
}

#[derive(Debug, Snafu)]
pub enum MockClientError {
    #[snafu(display("reading the current working directory failed"))]
    ReadCurrentDirectory { source: std::io::Error },

    #[snafu(display("connecting to the ACP server at {address} failed"))]
    Connect {
        source: std::io::Error,
        address: String,
    },

    #[snafu(display("initializing the ACP client failed: {source}"))]
    Initialize { source: acp::Error },

    #[snafu(display("authenticating with the ACP agent failed: {source}"))]
    Authenticate { source: acp::Error },

    #[snafu(display("creating an ACP session failed: {source}"))]
    CreateSession { source: acp::Error },

    #[snafu(display("sending the ACP prompt failed: {source}"))]
    SendPrompt { source: acp::Error },

    #[snafu(display("sending the ACP cancel notification failed: {source}"))]
    SendCancel { source: acp::Error },

    #[snafu(display("driving the ACP connection failed: {source}"))]
    ConnectionClosed { source: acp::Error },

    #[snafu(display("building the ACP runtime failed"))]
    BuildRuntime { source: std::io::Error },

    #[snafu(display("joining the ACP task failed"))]
    JoinTask { source: tokio::task::JoinError },

    #[snafu(display("the ACP request timed out after {timeout:?}"))]
    TimedOut { timeout: Duration },

    #[snafu(display("coordinating the prompt turn failed: {message}"))]
    TurnRuntime { message: String },

    #[snafu(display("the agent requested permission without both allow and deny options"))]
    InvalidPermissionOptions,

    #[snafu(display("the agent requested permission options the CLI cannot represent safely"))]
    UnsupportedPermissionOptions,
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
            default_working_dir: working_dir,
            session_working_dirs: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            session_acp_addresses: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            upstream_sessions: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            session_locks: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        })
    }

    pub async fn request_reply(&self, turn: TurnHandle) -> Result<ReplyResult> {
        let backend_session_id = turn.session_id().to_string();
        let tracked_turn = turn.clone();
        let result = self
            .run_locked_operation(
                backend_session_id.clone(),
                move |mock_address, working_dir, request_timeout, upstream_sessions| {
                    drive_acp_roundtrip_blocking(
                        mock_address,
                        working_dir,
                        turn,
                        request_timeout,
                        upstream_sessions,
                    )
                },
            )
            .await;

        if !tracked_turn.is_active().await {
            MockClient::forget_session(self, &backend_session_id).await;
        }

        result
    }

    pub async fn prime_session_hint(&self, backend_session_id: &str) -> Result<Option<String>> {
        let backend_session_id = backend_session_id.to_string();
        self.run_locked_operation(
            backend_session_id.clone(),
            move |mock_address, working_dir, request_timeout, upstream_sessions| {
                drive_acp_session_prime_blocking(
                    mock_address,
                    working_dir,
                    backend_session_id,
                    request_timeout,
                    upstream_sessions,
                )
            },
        )
        .await
    }

    async fn forget_session(&self, backend_session_id: &str) {
        self.session_working_dirs
            .lock()
            .await
            .remove(backend_session_id);
        self.upstream_sessions
            .lock()
            .await
            .remove(backend_session_id);
        self.session_acp_addresses
            .lock()
            .await
            .remove(backend_session_id);
        self.session_locks.lock().await.remove(backend_session_id);
    }

    async fn session_lock(&self, backend_session_id: &str) -> SessionLock {
        let mut session_locks = self.session_locks.lock().await;
        session_locks
            .entry(backend_session_id.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }

    async fn run_locked_operation<T, F>(
        &self,
        backend_session_id: String,
        operation: F,
    ) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(String, PathBuf, Duration, UpstreamSessions) -> Result<T> + Send + 'static,
    {
        let mock_address = self.session_acp_address(&backend_session_id).await;
        let working_dir = self.session_working_dir(&backend_session_id).await;
        let request_timeout = self.request_timeout;
        let upstream_sessions = self.upstream_sessions.clone();
        let session_lock = self.session_lock(&backend_session_id).await;
        let _serial = session_lock.lock().await;

        tokio::task::spawn_blocking(move || {
            operation(
                mock_address,
                working_dir,
                request_timeout,
                upstream_sessions,
            )
        })
        .await
        .context(JoinTaskSnafu)?
    }

    #[cfg(test)]
    async fn mapped_session_id(&self, backend_session_id: &str) -> Option<String> {
        self.upstream_sessions
            .lock()
            .await
            .get(backend_session_id)
            .cloned()
    }

    async fn session_working_dir(&self, backend_session_id: &str) -> PathBuf {
        self.session_working_dirs
            .lock()
            .await
            .get(backend_session_id)
            .cloned()
            .unwrap_or_else(|| self.default_working_dir.clone())
    }

    async fn session_acp_address(&self, backend_session_id: &str) -> String {
        self.session_acp_addresses
            .lock()
            .await
            .get(backend_session_id)
            .cloned()
            .unwrap_or_else(|| self.mock_address.clone())
    }
}

impl ReplyProvider for MockClient {
    fn request_reply<'a>(&'a self, turn: TurnHandle) -> ReplyFuture<'a> {
        Box::pin(MockClient::request_reply(self, turn))
    }

    fn bind_session<'a>(
        &'a self,
        session_id: &'a str,
        working_dir: PathBuf,
    ) -> BindSessionFuture<'a> {
        Box::pin(async move {
            self.session_working_dirs
                .lock()
                .await
                .insert(session_id.to_string(), working_dir);
            Ok(())
        })
    }

    fn prime_session<'a>(&'a self, session_id: &'a str) -> PrimeSessionFuture<'a> {
        Box::pin(MockClient::prime_session_hint(self, session_id))
    }

    fn bind_session_launch_metadata<'a>(
        &'a self,
        session_id: &'a str,
        metadata: AgentLaunchMetadata,
    ) -> BindLaunchMetadataFuture<'a> {
        Box::pin(async move {
            if let Some(address) = metadata.acp_address {
                self.session_acp_addresses
                    .lock()
                    .await
                    .insert(session_id.to_string(), address);
            }
            Ok(())
        })
    }

    fn forget_session(&self, session_id: &str) {
        let client = self.clone();
        let session_id = session_id.to_string();
        tokio::spawn(async move {
            MockClient::forget_session(&client, &session_id).await;
        });
    }
}

async fn drive_acp_roundtrip(
    mock_address: String,
    working_dir: PathBuf,
    turn: TurnHandle,
    request_timeout: Duration,
    upstream_sessions: UpstreamSessions,
) -> Result<ReplyResult> {
    let backend_session_id = turn.session_id().to_string();
    let client = BackendAcpClient::new(turn.clone());

    drive_acp_operation(
        mock_address,
        working_dir,
        request_timeout,
        backend_session_id,
        upstream_sessions,
        client,
        Box::new(
            move |conn,
                  working_dir,
                  backend_session_id,
                  client,
                  upstream_sessions,
                  capabilities| {
                Box::pin(async move {
                    let mut cancel_rx = turn.start_turn().await.map_err(session_runtime_error)?;
                    let session_id = load_or_create_session(
                        &conn,
                        &working_dir,
                        &backend_session_id,
                        &upstream_sessions,
                        capabilities.load_session,
                    )
                    .await?;
                    if *cancel_rx.borrow() {
                        return Ok(ReplyResult::Status("turn cancelled".to_string()));
                    }

                    prompt_session(
                        &conn,
                        &client,
                        &mut cancel_rx,
                        session_id,
                        turn.prompt_text(),
                    )
                    .await
                })
            },
        ),
    )
    .await
}

async fn drive_acp_session_prime(
    mock_address: String,
    working_dir: PathBuf,
    backend_session_id: String,
    request_timeout: Duration,
    upstream_sessions: UpstreamSessions,
) -> Result<Option<String>> {
    drive_acp_operation(
        mock_address,
        working_dir,
        request_timeout,
        backend_session_id,
        upstream_sessions,
        BackendAcpClient::without_turn(),
        Box::new(
            move |conn,
                  working_dir,
                  backend_session_id,
                  client,
                  upstream_sessions,
                  capabilities| {
                Box::pin(async move {
                    let session_id = load_or_create_session(
                        &conn,
                        &working_dir,
                        &backend_session_id,
                        &upstream_sessions,
                        capabilities.load_session,
                    )
                    .await?;
                    let _ = session_id;
                    let reply = client.take_reply_text();
                    Ok((!reply.is_empty()).then_some(reply))
                })
            },
        ),
    )
    .await
}

async fn connect_stream(mock_address: &str, retry_for: Duration) -> Result<TcpStream> {
    let deadline = Instant::now() + retry_for;
    let source = loop {
        match TcpStream::connect(mock_address).await {
            Ok(stream) => return Ok(stream),
            Err(error) if retry_for.is_zero() || Instant::now() >= deadline => break error,
            Err(_) => {}
        }
        tokio::time::sleep(next_connect_retry_delay(deadline)).await;
    };
    Err(MockClientError::Connect {
        source,
        address: mock_address.to_string(),
    })
}

fn next_connect_retry_delay(deadline: Instant) -> Duration {
    deadline
        .saturating_duration_since(Instant::now())
        .min(CONNECT_RETRY_DELAY)
}

fn connect_retry_budget(request_timeout: Duration) -> Duration {
    (request_timeout / 2).min(CONNECT_RETRY_MAX)
}

async fn prompt_session(
    conn: &acp::ConnectionTo<acp::Agent>,
    client: &BackendAcpClient,
    cancel_rx: &mut tokio::sync::watch::Receiver<bool>,
    session_id: String,
    prompt: &str,
) -> Result<ReplyResult> {
    let prompt_future = conn
        .send_request(schema::PromptRequest::new(
            session_id.clone(),
            vec![prompt.to_string().into()],
        ))
        .block_task();
    let send_cancel = |cancel_request| conn.send_notification(cancel_request);
    await_prompt_reply(
        prompt_future,
        cancel_rx,
        schema::CancelNotification::new(session_id),
        send_cancel,
        client,
    )
    .await
}

async fn await_prompt_reply<PromptFut, CancelFn>(
    prompt_future: PromptFut,
    cancel_rx: &mut tokio::sync::watch::Receiver<bool>,
    cancel_request: schema::CancelNotification,
    send_cancel: CancelFn,
    client: &BackendAcpClient,
) -> Result<ReplyResult>
where
    PromptFut: Future<Output = acp::Result<schema::PromptResponse>>,
    CancelFn: FnOnce(schema::CancelNotification) -> Result<(), acp::Error>,
{
    tokio::pin!(prompt_future);
    tokio::select! {
        response = &mut prompt_future => reply_from_prompt_response(response, client),
        changed = cancel_rx.changed() => {
            let cancelled = changed.is_ok() && *cancel_rx.borrow();
            handle_cancelled_prompt(cancelled, &mut prompt_future, cancel_request, send_cancel, client).await
        }
    }
}

fn reply_from_prompt_response(
    response: acp::Result<schema::PromptResponse>,
    client: &BackendAcpClient,
) -> Result<ReplyResult> {
    Ok(reply_from_stop_reason(
        response.context(SendPromptSnafu)?.stop_reason,
        client.reply_text(),
    ))
}

async fn handle_cancelled_prompt<PromptFut, CancelFn>(
    cancelled: bool,
    prompt_future: &mut Pin<&mut PromptFut>,
    cancel_request: schema::CancelNotification,
    send_cancel: CancelFn,
    client: &BackendAcpClient,
) -> Result<ReplyResult>
where
    PromptFut: Future<Output = acp::Result<schema::PromptResponse>>,
    CancelFn: FnOnce(schema::CancelNotification) -> Result<(), acp::Error>,
{
    if cancelled {
        send_cancel(cancel_request).context(SendCancelSnafu)?;
        let _ = prompt_future.await;
        return Ok(ReplyResult::Status("turn cancelled".to_string()));
    }

    reply_from_prompt_response(prompt_future.await, client)
}

async fn respond_permission_request(
    client: BackendAcpClient,
    args: schema::RequestPermissionRequest,
    responder: acp::Responder<schema::RequestPermissionResponse>,
    connection: acp::ConnectionTo<acp::Agent>,
) -> std::result::Result<(), acp::Error> {
    connection.spawn(async move {
        let result = client.request_permission(args).await;
        responder.respond_with_result(result)?;
        Ok(())
    })?;
    Ok(())
}

async fn forward_session_notification(
    client: BackendAcpClient,
    args: schema::SessionNotification,
) -> std::result::Result<(), acp::Error> {
    client.session_notification(args).await
}

struct BackendIo {
    outgoing: DynWr,
    incoming: DynRd,
}

impl BackendIo {
    fn new(reader: OwnedReadHalf, writer: OwnedWriteHalf) -> Self {
        Self {
            outgoing: Box::new(writer.compat_write()) as DynWr,
            incoming: Box::new(reader.compat()) as DynRd,
        }
    }
}

async fn write_backend_io_line(mut writer: DynWr, line: String) -> std::io::Result<DynWr> {
    let mut bytes = line.into_bytes();
    bytes.push(b'\n');
    writer.write_all(&bytes).await?;
    Ok(writer)
}

impl<R: acp::Role> ConnectTo<R> for BackendIo {
    async fn connect_to(
        self,
        client: impl ConnectTo<R::Counterpart>,
    ) -> std::result::Result<(), acp::Error> {
        let (channel, serve_io) = <BackendIo as ConnectTo<R>>::into_channel_and_future(self);
        let client = acp::DynConnectTo::new(client);
        let serve_client: BoxFuture<'static, std::result::Result<(), acp::Error>> =
            Box::pin(client.connect_to(channel));
        match future::select(serve_client, serve_io).await {
            Either::Left((result, _)) | Either::Right((result, _)) => result,
        }
    }

    fn into_channel_and_future(
        self,
    ) -> (
        acp::Channel,
        BoxFuture<'static, std::result::Result<(), acp::Error>>,
    ) {
        let Self { outgoing, incoming } = self;
        let incoming_lines: LineStream = Box::pin(BufReader::new(incoming).lines());
        let outgoing_sink: LineSink = Box::pin(unfold(outgoing, write_backend_io_line));
        ConnectTo::<R>::into_channel_and_future(acp::Lines::new(outgoing_sink, incoming_lines))
    }
}

#[derive(Clone)]
struct BackendDispatchHandler {
    request_client: BackendAcpClient,
    notification_client: BackendAcpClient,
}

impl BackendDispatchHandler {
    fn new(request_client: BackendAcpClient, notification_client: BackendAcpClient) -> Self {
        Self {
            request_client,
            notification_client,
        }
    }

    async fn handle_permission_request(
        &self,
        dispatch: acp::Dispatch,
        connection: acp::ConnectionTo<acp::Agent>,
    ) -> std::result::Result<Option<acp::Dispatch>, acp::Error> {
        match dispatch.into_request::<schema::RequestPermissionRequest>()? {
            Ok((args, responder)) => {
                respond_permission_request(
                    self.request_client.clone(),
                    args,
                    responder,
                    connection,
                )
                .await?;
                Ok(None)
            }
            Err(dispatch) => Ok(Some(dispatch)),
        }
    }

    async fn handle_session_notification(
        &self,
        dispatch: acp::Dispatch,
    ) -> std::result::Result<Option<acp::Dispatch>, acp::Error> {
        match dispatch.into_notification::<schema::SessionNotification>()? {
            Ok(args) => {
                forward_session_notification(self.notification_client.clone(), args).await?;
                Ok(None)
            }
            Err(dispatch) => Ok(Some(dispatch)),
        }
    }
}

impl acp::HandleDispatchFrom<acp::Agent> for BackendDispatchHandler {
    fn describe_chain(&self) -> impl std::fmt::Debug {
        "BackendDispatchHandler"
    }

    async fn handle_dispatch_from(
        &mut self,
        dispatch: acp::Dispatch,
        connection: acp::ConnectionTo<acp::Agent>,
    ) -> std::result::Result<acp::Handled<acp::Dispatch>, acp::Error> {
        let Some(dispatch) = self
            .handle_permission_request(dispatch, connection.clone())
            .await?
        else {
            return Ok(acp::Handled::Yes);
        };
        let Some(dispatch) = self.handle_session_notification(dispatch).await? else {
            return Ok(acp::Handled::Yes);
        };
        Ok(acp::Handled::No {
            message: dispatch,
            retry: false,
        })
    }
}

fn build_backend_client_builder(
    request_client: BackendAcpClient,
    notification_client: BackendAcpClient,
) -> acp::Builder<acp::Client, BackendDispatchHandler, acp::NullRun> {
    acp::Builder::new_with(
        acp::Client,
        BackendDispatchHandler::new(request_client, notification_client),
    )
    .name("acp-web-backend-mock-client")
}

async fn run_connected_operation<T: 'static>(
    conn: acp::ConnectionTo<acp::Agent>,
    working_dir: PathBuf,
    backend_session_id: String,
    client: BackendAcpClient,
    upstream_sessions: UpstreamSessions,
    operation: ConnectedOperation<T>,
) -> Result<T> {
    let capabilities = initialize_connection(&conn).await?;
    operation(
        conn,
        working_dir,
        backend_session_id,
        client,
        upstream_sessions,
        capabilities,
    )
    .await
}

async fn connect_backend_mock_client<T: 'static>(
    reader: OwnedReadHalf,
    writer: OwnedWriteHalf,
    request_client: BackendAcpClient,
    notification_client: BackendAcpClient,
    connected_main: ConnectedMainState<T>,
) -> std::result::Result<Result<T>, acp::Error> {
    build_backend_client_builder(request_client, notification_client)
        .connect_with(
            acp::DynConnectTo::new(BackendIo::new(reader, writer)),
            move |conn| run_connected_main(conn, connected_main),
        )
        .await
}

fn drive_acp_roundtrip_blocking(
    mock_address: String,
    working_dir: PathBuf,
    turn: TurnHandle,
    request_timeout: Duration,
    upstream_sessions: UpstreamSessions,
) -> Result<ReplyResult> {
    drive_acp_operation_blocking(
        request_timeout,
        drive_acp_roundtrip(
            mock_address,
            working_dir,
            turn,
            request_timeout,
            upstream_sessions,
        ),
    )
}

fn drive_acp_session_prime_blocking(
    mock_address: String,
    working_dir: PathBuf,
    backend_session_id: String,
    request_timeout: Duration,
    upstream_sessions: UpstreamSessions,
) -> Result<Option<String>> {
    drive_acp_operation_blocking(
        request_timeout,
        drive_acp_session_prime(
            mock_address,
            working_dir,
            backend_session_id,
            request_timeout,
            upstream_sessions,
        ),
    )
}

fn drive_acp_operation_blocking<T, Fut>(request_timeout: Duration, operation: Fut) -> Result<T>
where
    Fut: Future<Output = Result<T>>,
{
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context(BuildRuntimeSnafu)?;

    runtime.block_on(async move {
        tokio::time::timeout(request_timeout, operation)
            .await
            .map_err(|_| MockClientError::TimedOut {
                timeout: request_timeout,
            })?
    })
}

async fn drive_acp_operation<T: 'static>(
    mock_address: String,
    working_dir: PathBuf,
    request_timeout: Duration,
    backend_session_id: String,
    upstream_sessions: UpstreamSessions,
    client: BackendAcpClient,
    operation: ConnectedOperation<T>,
) -> Result<T> {
    let stream = connect_stream(&mock_address, connect_retry_budget(request_timeout)).await?;
    let (reader, writer) = stream.into_split();
    let request_client = client.clone();
    let notification_client = client.clone();

    let connected_main = ConnectedMainState {
        working_dir,
        backend_session_id,
        client,
        upstream_sessions,
        operation,
    };

    connect_backend_mock_client(
        reader,
        writer,
        request_client,
        notification_client,
        connected_main,
    )
    .await
    .context(ConnectionClosedSnafu)?
}

async fn run_connected_main<T: 'static>(
    conn: acp::ConnectionTo<acp::Agent>,
    state: ConnectedMainState<T>,
) -> std::result::Result<Result<T>, acp::Error> {
    let ConnectedMainState {
        working_dir,
        backend_session_id,
        client,
        upstream_sessions,
        operation,
    } = state;

    Ok(run_connected_operation(
        conn,
        working_dir,
        backend_session_id,
        client,
        upstream_sessions,
        operation,
    )
    .await)
}

async fn initialize_connection(
    conn: &acp::ConnectionTo<acp::Agent>,
) -> Result<AgentConnectionCapabilities> {
    let response = conn
        .send_request(
            schema::InitializeRequest::new(schema::ProtocolVersion::V1).client_info(
                schema::Implementation::new("acp-web-backend", env!("CARGO_PKG_VERSION"))
                    .title("ACP Web Backend"),
            ),
        )
        .block_task()
        .await
        .context(InitializeSnafu)?;
    authenticate_with_agent(conn, &response.auth_methods).await?;
    Ok(AgentConnectionCapabilities {
        load_session: response.agent_capabilities.load_session,
    })
}

async fn authenticate_with_agent(
    conn: &acp::ConnectionTo<acp::Agent>,
    auth_methods: &[schema::AuthMethod],
) -> Result<()> {
    let Some(method) = auth_methods.first() else {
        return Ok(());
    };
    conn.send_request(schema::AuthenticateRequest::new(method.id().clone()))
        .block_task()
        .await
        .context(AuthenticateSnafu)?;
    Ok(())
}

fn session_runtime_error(source: crate::sessions::SessionStoreError) -> MockClientError {
    MockClientError::TurnRuntime {
        message: source.message().to_string(),
    }
}

async fn load_or_create_session(
    conn: &acp::ConnectionTo<acp::Agent>,
    working_dir: &Path,
    backend_session_id: &str,
    upstream_sessions: &UpstreamSessions,
    can_load_session: bool,
) -> Result<String> {
    let cached_session_id = upstream_sessions
        .lock()
        .await
        .get(backend_session_id)
        .cloned();
    let load_succeeded = if can_load_session {
        load_cached_session(conn, working_dir, cached_session_id.as_ref()).await
    } else {
        false
    };
    let mut cached_sessions = upstream_sessions.lock().await;
    if let Some(session_id) = reuse_cached_session(
        cached_session_id,
        load_succeeded,
        &mut cached_sessions,
        backend_session_id,
    ) {
        return Ok(session_id);
    }
    drop(cached_sessions);

    let session = conn
        .send_request(schema::NewSessionRequest::new(working_dir.to_path_buf()))
        .block_task()
        .await
        .context(CreateSessionSnafu)?;
    let session_id = session.session_id.to_string();
    upstream_sessions
        .lock()
        .await
        .insert(backend_session_id.to_string(), session_id.clone());
    Ok(session_id)
}

async fn load_cached_session(
    conn: &acp::ConnectionTo<acp::Agent>,
    working_dir: &Path,
    cached_session_id: Option<&String>,
) -> bool {
    if let Some(session_id) = cached_session_id {
        conn.send_request(schema::LoadSessionRequest::new(
            session_id.clone(),
            working_dir.to_path_buf(),
        ))
        .block_task()
        .await
        .is_ok()
    } else {
        false
    }
}

fn reply_from_stop_reason(stop_reason: schema::StopReason, reply_text: String) -> ReplyResult {
    match stop_reason {
        schema::StopReason::Cancelled => ReplyResult::Status("turn cancelled".to_string()),
        _ if reply_text.is_empty() => ReplyResult::NoOutput,
        _ => ReplyResult::Reply(reply_text),
    }
}

fn reuse_cached_session(
    cached_session_id: Option<String>,
    load_succeeded: bool,
    upstream_sessions: &mut HashMap<String, String>,
    backend_session_id: &str,
) -> Option<String> {
    let session_id = cached_session_id?;
    if load_succeeded {
        return Some(session_id);
    }

    upstream_sessions.remove(backend_session_id);
    None
}
