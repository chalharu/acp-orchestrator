use std::{
    collections::HashMap,
    env,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::Arc,
    time::Duration,
};

use agent_client_protocol::{self as acp, Agent as _};
use snafu::prelude::*;
use tokio::net::TcpStream;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

mod backend_client;

#[cfg(test)]
mod tests;

use crate::sessions::TurnHandle;
use backend_client::BackendAcpClient;

type Result<T, E = MockClientError> = std::result::Result<T, E>;
pub type ReplyFuture<'a> = Pin<Box<dyn Future<Output = Result<ReplyResult>> + Send + 'a>>;
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplyResult {
    Reply(String),
    Status(String),
    NoOutput,
}

pub trait ReplyProvider: Send + Sync + std::fmt::Debug {
    fn request_reply<'a>(&'a self, turn: TurnHandle) -> ReplyFuture<'a>;

    fn forget_session(&self, _session_id: &str) {}
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

    #[snafu(display("connecting to the ACP server at {address} failed"))]
    Connect {
        source: std::io::Error,
        address: String,
    },

    #[snafu(display("initializing the ACP client failed"))]
    Initialize { source: acp::Error },

    #[snafu(display("creating an ACP session failed"))]
    CreateSession { source: acp::Error },

    #[snafu(display("loading an ACP session failed"))]
    LoadSession { source: acp::Error },

    #[snafu(display("sending the ACP prompt failed"))]
    SendPrompt { source: acp::Error },

    #[snafu(display("sending the ACP cancel notification failed"))]
    SendCancel { source: acp::Error },

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
            working_dir,
            upstream_sessions: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            session_locks: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        })
    }

    pub async fn request_reply(&self, turn: TurnHandle) -> Result<ReplyResult> {
        let backend_session_id = turn.session_id().to_string();
        let tracked_turn = turn.clone();
        let mock_address = self.mock_address.clone();
        let working_dir = self.working_dir.clone();
        let request_timeout = self.request_timeout;
        let upstream_sessions = self.upstream_sessions.clone();
        let session_lock = self.session_lock(&backend_session_id).await;
        let _serial = session_lock.lock().await;

        let result = tokio::task::spawn_blocking(move || {
            drive_acp_roundtrip_blocking(
                mock_address,
                working_dir,
                turn,
                request_timeout,
                upstream_sessions,
            )
        })
        .await
        .context(JoinTaskSnafu);

        if !tracked_turn.is_active().await {
            MockClient::forget_session(self, &backend_session_id).await;
        }

        result?
    }

    async fn forget_session(&self, backend_session_id: &str) {
        self.upstream_sessions
            .lock()
            .await
            .remove(backend_session_id);
        self.session_locks.lock().await.remove(backend_session_id);
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
    fn request_reply<'a>(&'a self, turn: TurnHandle) -> ReplyFuture<'a> {
        Box::pin(MockClient::request_reply(self, turn))
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
    upstream_sessions: Arc<tokio::sync::Mutex<HashMap<String, String>>>,
) -> Result<ReplyResult> {
    let backend_session_id = turn.session_id().to_string();
    let mut cancel_rx = turn.start_turn().await.map_err(session_runtime_error)?;
    let stream = connect_stream(&mock_address).await?;
    let client = BackendAcpClient::new(turn.clone());
    let local_set = tokio::task::LocalSet::new();

    local_set
        .run_until(run_roundtrip_on_connection(
            stream,
            working_dir,
            turn,
            backend_session_id,
            &mut cancel_rx,
            client,
            upstream_sessions,
        ))
        .await
}

async fn connect_stream(mock_address: &str) -> Result<TcpStream> {
    TcpStream::connect(mock_address)
        .await
        .context(ConnectSnafu {
            address: mock_address.to_string(),
        })
}

async fn run_roundtrip_on_connection(
    stream: TcpStream,
    working_dir: PathBuf,
    turn: TurnHandle,
    backend_session_id: String,
    cancel_rx: &mut tokio::sync::watch::Receiver<bool>,
    client: BackendAcpClient,
    upstream_sessions: Arc<tokio::sync::Mutex<HashMap<String, String>>>,
) -> Result<ReplyResult> {
    let (reader, writer) = stream.into_split();
    let (conn, handle_io) = acp::ClientSideConnection::new(
        client.clone(),
        writer.compat_write(),
        reader.compat(),
        |future| {
            tokio::task::spawn_local(future);
        },
    );
    let mut io_task = Some(tokio::task::spawn_local(handle_io));
    initialize_connection(&conn).await?;
    let session_id =
        load_or_create_session(&conn, &working_dir, &backend_session_id, &upstream_sessions)
            .await?;
    if let Some(reply) = cancelled_before_prompt_reply(cancel_rx, &mut io_task).await {
        return Ok(reply);
    }

    let reply = prompt_session(&conn, &client, cancel_rx, session_id, turn.prompt_text()).await;
    stop_io_task(io_task).await;
    reply
}

async fn prompt_session(
    conn: &acp::ClientSideConnection,
    client: &BackendAcpClient,
    cancel_rx: &mut tokio::sync::watch::Receiver<bool>,
    session_id: String,
    prompt: &str,
) -> Result<ReplyResult> {
    let prompt_future = conn.prompt(acp::PromptRequest::new(
        session_id.clone(),
        vec![prompt.to_string().into()],
    ));
    await_prompt_reply(
        prompt_future,
        cancel_rx,
        acp::CancelNotification::new(session_id),
        |cancel_request| conn.cancel(cancel_request),
        client,
    )
    .await
}

async fn await_prompt_reply<PromptFut, CancelFn, CancelFut>(
    prompt_future: PromptFut,
    cancel_rx: &mut tokio::sync::watch::Receiver<bool>,
    cancel_request: acp::CancelNotification,
    send_cancel: CancelFn,
    client: &BackendAcpClient,
) -> Result<ReplyResult>
where
    PromptFut: Future<Output = acp::Result<acp::PromptResponse>>,
    CancelFn: FnOnce(acp::CancelNotification) -> CancelFut,
    CancelFut: Future<Output = acp::Result<(), acp::Error>>,
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
    response: acp::Result<acp::PromptResponse>,
    client: &BackendAcpClient,
) -> Result<ReplyResult> {
    Ok(reply_from_stop_reason(
        response.context(SendPromptSnafu)?.stop_reason,
        client.reply_text(),
    ))
}

async fn handle_cancelled_prompt<PromptFut, CancelFn, CancelFut>(
    cancelled: bool,
    prompt_future: &mut Pin<&mut PromptFut>,
    cancel_request: acp::CancelNotification,
    send_cancel: CancelFn,
    client: &BackendAcpClient,
) -> Result<ReplyResult>
where
    PromptFut: Future<Output = acp::Result<acp::PromptResponse>>,
    CancelFn: FnOnce(acp::CancelNotification) -> CancelFut,
    CancelFut: Future<Output = acp::Result<(), acp::Error>>,
{
    if cancelled {
        send_cancel(cancel_request).await.context(SendCancelSnafu)?;
        let _ = prompt_future.await;
        return Ok(ReplyResult::Status("turn cancelled".to_string()));
    }

    reply_from_prompt_response(prompt_future.await, client)
}

async fn stop_io_task<T>(io_task: Option<tokio::task::JoinHandle<T>>) {
    let io_task = io_task.expect("io task should be available until prompt handling ends");
    io_task.abort();
    let _ = io_task.await;
}

fn drive_acp_roundtrip_blocking(
    mock_address: String,
    working_dir: PathBuf,
    turn: TurnHandle,
    request_timeout: Duration,
    upstream_sessions: Arc<tokio::sync::Mutex<HashMap<String, String>>>,
) -> Result<ReplyResult> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context(BuildRuntimeSnafu)?;

    runtime.block_on(async move {
        tokio::time::timeout(
            request_timeout,
            drive_acp_roundtrip(mock_address, working_dir, turn, upstream_sessions),
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

fn session_runtime_error(source: crate::sessions::SessionStoreError) -> MockClientError {
    MockClientError::TurnRuntime {
        message: source.message().to_string(),
    }
}

async fn load_or_create_session(
    conn: &acp::ClientSideConnection,
    working_dir: &Path,
    backend_session_id: &str,
    upstream_sessions: &Arc<tokio::sync::Mutex<HashMap<String, String>>>,
) -> Result<String> {
    let cached_session_id = upstream_sessions
        .lock()
        .await
        .get(backend_session_id)
        .cloned();
    let load_succeeded = if let Some(session_id) = cached_session_id.as_ref() {
        conn.load_session(acp::LoadSessionRequest::new(
            session_id.clone(),
            working_dir.to_path_buf(),
        ))
        .await
        .is_ok()
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
        .new_session(acp::NewSessionRequest::new(working_dir.to_path_buf()))
        .await
        .context(CreateSessionSnafu)?;
    let session_id = session.session_id.to_string();
    upstream_sessions
        .lock()
        .await
        .insert(backend_session_id.to_string(), session_id.clone());
    Ok(session_id)
}

fn reply_from_stop_reason(stop_reason: acp::StopReason, reply_text: String) -> ReplyResult {
    match stop_reason {
        acp::StopReason::Cancelled => ReplyResult::Status("turn cancelled".to_string()),
        _ if reply_text.is_empty() => ReplyResult::NoOutput,
        _ => ReplyResult::Reply(reply_text),
    }
}

async fn cancelled_before_prompt<T>(io_task: tokio::task::JoinHandle<T>) -> ReplyResult {
    io_task.abort();
    let _ = io_task.await;
    ReplyResult::Status("turn cancelled".to_string())
}

async fn cancelled_before_prompt_reply<T>(
    cancel_rx: &tokio::sync::watch::Receiver<bool>,
    io_task: &mut Option<tokio::task::JoinHandle<T>>,
) -> Option<ReplyResult> {
    if !*cancel_rx.borrow() {
        return None;
    }

    let io_task = io_task
        .take()
        .expect("io task should be available while checking cancellation");
    Some(cancelled_before_prompt(io_task).await)
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
