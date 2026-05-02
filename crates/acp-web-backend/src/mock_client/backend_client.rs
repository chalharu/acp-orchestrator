use super::{
    InvalidPermissionOptionsSnafu, MockClientError, Result, UnsupportedPermissionOptionsSnafu,
};
use crate::{
    agent_runtime::{
        AGENT_RUNTIMES_DIR_NAME, CHROOT_CHECKOUT_ROOT, DEFAULT_AGENT_RUN_GID, DEFAULT_AGENT_RUN_UID,
    },
    contract_permissions::ToolCallMetadata,
    sessions::{PermissionResolutionOutcome, TurnHandle},
};
use agent_client_protocol::{self as acp, schema};
#[cfg(unix)]
use std::os::{
    fd::{AsRawFd, FromRawFd, OwnedFd},
    unix::ffi::OsStrExt,
};
use std::{
    collections::HashMap,
    ffi::{CString, OsStr},
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    process::Stdio,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::AsyncReadExt,
    process::{Child, Command},
};

const PROMPT_RESPONSE_NOTIFICATION_GRACE: Duration = Duration::from_millis(100);
const PROMPT_RESPONSE_NOTIFICATION_IDLE: Duration = Duration::from_millis(10);
const DEFAULT_TERMINAL_OUTPUT_BYTE_LIMIT: usize = 64 * 1024;
const MAX_TERMINAL_OUTPUT_BYTE_LIMIT: usize = 1024 * 1024;
#[cfg(test)]
const TEST_SKIP_CHROOT_PREEXEC_MARKER: &str = ".acp-test-skip-chroot-preexec";

#[derive(Debug, Clone)]
pub(super) struct BackendAcpClient {
    turn: Option<TurnHandle>,
    collected: Arc<Mutex<String>>,
    reply_notify: Arc<tokio::sync::Notify>,
    streaming_enabled: Arc<AtomicBool>,
    collect_while_muted: bool,
    runtime: Arc<RuntimeToolContext>,
}

impl BackendAcpClient {
    #[cfg(test)]
    pub(super) fn new(turn: TurnHandle) -> Self {
        Self::with_streaming(turn, true, false)
    }

    #[cfg(test)]
    pub(super) fn new_muted(turn: TurnHandle) -> Self {
        Self::with_streaming(turn, false, false)
    }

    pub(super) fn new_muted_with_checkout(turn: TurnHandle, checkout_root: PathBuf) -> Self {
        Self::with_streaming_and_checkout(turn, false, false, checkout_root)
    }

    #[cfg(test)]
    fn with_streaming(
        turn: TurnHandle,
        streaming_enabled: bool,
        collect_while_muted: bool,
    ) -> Self {
        let checkout_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self::with_streaming_and_checkout(
            turn,
            streaming_enabled,
            collect_while_muted,
            checkout_root,
        )
    }

    fn with_streaming_and_checkout(
        turn: TurnHandle,
        streaming_enabled: bool,
        collect_while_muted: bool,
        checkout_root: PathBuf,
    ) -> Self {
        Self {
            turn: Some(turn),
            collected: Arc::new(Mutex::new(String::new())),
            reply_notify: Arc::new(tokio::sync::Notify::new()),
            streaming_enabled: Arc::new(AtomicBool::new(streaming_enabled)),
            collect_while_muted,
            runtime: Arc::new(RuntimeToolContext::new(checkout_root)),
        }
    }

    #[cfg(test)]
    pub(super) fn without_turn() -> Self {
        Self::without_turn_with_checkout(
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        )
    }

    pub(super) fn without_turn_with_checkout(checkout_root: PathBuf) -> Self {
        Self {
            turn: None,
            collected: Arc::new(Mutex::new(String::new())),
            reply_notify: Arc::new(tokio::sync::Notify::new()),
            streaming_enabled: Arc::new(AtomicBool::new(false)),
            collect_while_muted: true,
            runtime: Arc::new(RuntimeToolContext::new(checkout_root)),
        }
    }

    pub(super) fn supports_chroot_terminal_for_checkout(checkout_root: &Path) -> bool {
        RuntimeToolContext::supports_chroot_terminal(checkout_root)
    }

    pub(super) fn reply_text(&self) -> String {
        self.collected
            .lock()
            .expect("mock reply buffer mutex should not be poisoned")
            .clone()
    }

    pub(super) fn take_reply_text(&self) -> String {
        std::mem::take(
            &mut *self
                .collected
                .lock()
                .expect("mock reply buffer mutex should not be poisoned"),
        )
    }

    pub(super) async fn reply_text_after_notifications(&self) -> String {
        self.wait_for_response_notifications(prompt_response_notification_deadline())
            .await;
        self.reply_text()
    }

    pub(super) async fn take_reply_text_after_notifications(&self) -> String {
        self.wait_for_response_notifications(prompt_response_notification_deadline())
            .await;
        self.take_reply_text()
    }

    pub(super) fn enable_streaming(&self) {
        self.streaming_enabled.store(true, Ordering::Release);
    }

    #[cfg(test)]
    pub(super) async fn wait_for_response_notifications_until_for_test(
        &self,
        deadline: tokio::time::Instant,
    ) {
        self.wait_for_response_notifications(deadline).await;
    }

    async fn wait_for_response_notifications(&self, deadline: tokio::time::Instant) {
        loop {
            let Some(wait_for) = self.next_notification_wait(deadline) else {
                return;
            };
            if tokio::time::timeout(wait_for, self.reply_notify.notified())
                .await
                .is_err()
            {
                return;
            }
        }
    }

    fn next_notification_wait(&self, deadline: tokio::time::Instant) -> Option<Duration> {
        let remaining = deadline.checked_duration_since(tokio::time::Instant::now())?;
        Some(if self.reply_text().is_empty() {
            remaining
        } else {
            remaining.min(PROMPT_RESPONSE_NOTIFICATION_IDLE)
        })
    }

    pub(super) async fn request_permission(
        &self,
        args: schema::RequestPermissionRequest,
    ) -> acp::Result<schema::RequestPermissionResponse> {
        let turn = self.turn.clone().ok_or_else(acp::Error::internal_error)?;
        let (approve_option_id, deny_option_id) =
            permission_option_ids(&args).map_err(|_| acp::Error::invalid_params())?;
        let summary = args
            .tool_call
            .fields
            .title
            .as_ref()
            .map(|title| self.runtime.sanitize_text(title))
            .unwrap_or_else(|| format!("tool {}", args.tool_call.tool_call_id));
        let tool_call = Some(self.tool_call_update_metadata(&args.tool_call));
        let resolution = turn
            .register_permission_request(summary, tool_call, approve_option_id, deny_option_id)
            .await
            .map_err(to_acp_error)?;

        match resolution.wait().await {
            PermissionResolutionOutcome::Selected(option_id) => Ok(
                schema::RequestPermissionResponse::new(schema::RequestPermissionOutcome::Selected(
                    schema::SelectedPermissionOutcome::new(option_id),
                )),
            ),
            PermissionResolutionOutcome::Cancelled => Ok(schema::RequestPermissionResponse::new(
                schema::RequestPermissionOutcome::Cancelled,
            )),
        }
    }

    pub(super) async fn session_notification(
        &self,
        args: schema::SessionNotification,
    ) -> acp::Result<()> {
        match args.update {
            schema::SessionUpdate::AgentMessageChunk(chunk) => {
                let text = content_text(chunk.content);
                let streaming_enabled = self.streaming_enabled.load(Ordering::Acquire);
                if streaming_enabled || self.collect_while_muted {
                    self.collected
                        .lock()
                        .expect("mock reply buffer mutex should not be poisoned")
                        .push_str(&text);
                    self.reply_notify.notify_one();
                }
                if streaming_enabled && let Some(turn) = self.turn.as_ref() {
                    turn.stream_assistant_chunk(text)
                        .await
                        .map_err(to_acp_error)?;
                }
            }
            schema::SessionUpdate::ToolCall(call) => {
                if let Some(turn) = self.turn.as_ref() {
                    turn.stream_tool_call(self.tool_call_metadata(&call))
                        .await
                        .map_err(to_acp_error)?;
                }
            }
            schema::SessionUpdate::ToolCallUpdate(update) => {
                if let Some(turn) = self.turn.as_ref() {
                    turn.stream_tool_call_update(self.tool_call_update_metadata(&update))
                        .await
                        .map_err(to_acp_error)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub(super) async fn read_text_file(
        &self,
        args: schema::ReadTextFileRequest,
    ) -> acp::Result<schema::ReadTextFileResponse> {
        let mut file = self.runtime.open_read_file(&args.path)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|_| tool_error("reading file failed"))?;
        let text = String::from_utf8(bytes).map_err(|_| tool_error("file is not valid UTF-8"))?;
        let start = args.line.unwrap_or(1);
        if start == 0 {
            return Err(tool_error("line must be 1-based"));
        }
        let mut lines = text.lines().skip((start - 1) as usize);
        let content = match args.limit {
            Some(limit) => lines
                .by_ref()
                .take(limit as usize)
                .collect::<Vec<_>>()
                .join("\n"),
            None => lines.collect::<Vec<_>>().join("\n"),
        };
        Ok(schema::ReadTextFileResponse::new(content))
    }

    pub(super) async fn write_text_file(
        &self,
        args: schema::WriteTextFileRequest,
    ) -> acp::Result<schema::WriteTextFileResponse> {
        let mut file = self.runtime.open_write_file(&args.path)?;
        file.write_all(args.content.as_bytes())
            .map_err(|_| tool_error("writing file failed"))?;
        Ok(schema::WriteTextFileResponse::new())
    }

    pub(super) async fn create_terminal(
        &self,
        args: schema::CreateTerminalRequest,
    ) -> acp::Result<schema::CreateTerminalResponse> {
        self.runtime.create_terminal(args).await
    }

    pub(super) async fn terminal_output(
        &self,
        args: schema::TerminalOutputRequest,
    ) -> acp::Result<schema::TerminalOutputResponse> {
        self.runtime.terminal_output(args).await
    }

    pub(super) async fn wait_for_terminal_exit(
        &self,
        args: schema::WaitForTerminalExitRequest,
    ) -> acp::Result<schema::WaitForTerminalExitResponse> {
        self.runtime.wait_for_terminal_exit(args).await
    }

    pub(super) async fn kill_terminal(
        &self,
        args: schema::KillTerminalRequest,
    ) -> acp::Result<schema::KillTerminalResponse> {
        self.runtime.kill_terminal(args).await
    }

    pub(super) async fn release_terminal(
        &self,
        args: schema::ReleaseTerminalRequest,
    ) -> acp::Result<schema::ReleaseTerminalResponse> {
        self.runtime.release_terminal(args).await
    }

    fn tool_call_metadata(&self, call: &schema::ToolCall) -> ToolCallMetadata {
        ToolCallMetadata {
            tool_call_id: call.tool_call_id.to_string(),
            title: Some(self.runtime.sanitize_text(&call.title)),
            kind: Some(tool_kind_name(call.kind).to_string()),
            status: Some(tool_status_name(call.status).to_string()),
            raw_input: call
                .raw_input
                .clone()
                .map(|value| self.runtime.sanitize_json_value(value)),
            raw_output: call
                .raw_output
                .clone()
                .map(|value| self.runtime.sanitize_json_value(value)),
        }
    }

    fn tool_call_update_metadata(&self, update: &schema::ToolCallUpdate) -> ToolCallMetadata {
        ToolCallMetadata {
            tool_call_id: update.tool_call_id.to_string(),
            title: update
                .fields
                .title
                .as_ref()
                .map(|title| self.runtime.sanitize_text(title)),
            kind: update
                .fields
                .kind
                .map(|kind| tool_kind_name(kind).to_string()),
            status: update
                .fields
                .status
                .map(|status| tool_status_name(status).to_string()),
            raw_input: update
                .fields
                .raw_input
                .clone()
                .map(|value| self.runtime.sanitize_json_value(value)),
            raw_output: update
                .fields
                .raw_output
                .clone()
                .map(|value| self.runtime.sanitize_json_value(value)),
        }
    }
}

#[derive(Debug)]
struct RuntimeToolContext {
    checkout_root: PathBuf,
    terminal_boundary: Option<TerminalBoundary>,
    terminals: tokio::sync::Mutex<TerminalRegistry>,
}

#[derive(Debug, Clone)]
struct TerminalBoundary {
    root_dir: PathBuf,
    run_uid: u32,
    run_gid: u32,
}

#[derive(Debug, Default)]
struct TerminalRegistry {
    next_id: u64,
    entries: HashMap<String, Arc<tokio::sync::Mutex<TerminalEntry>>>,
}

#[derive(Debug)]
struct TerminalEntry {
    child: Option<Arc<tokio::sync::Mutex<Child>>>,
    child_pid: Option<u32>,
    output_drains: Vec<tokio::task::JoinHandle<()>>,
    output: String,
    truncated: bool,
    output_limit: usize,
    exit_status: Option<schema::TerminalExitStatus>,
}

impl RuntimeToolContext {
    fn new(checkout_root: PathBuf) -> Self {
        let terminal_boundary = terminal_boundary_for_checkout(&checkout_root);
        Self {
            checkout_root,
            terminal_boundary,
            terminals: tokio::sync::Mutex::new(TerminalRegistry::default()),
        }
    }

    pub(super) fn supports_chroot_terminal(checkout_root: &Path) -> bool {
        terminal_boundary_for_checkout(checkout_root).is_some()
    }

    fn checkout_root(&self) -> acp::Result<PathBuf> {
        self.checkout_root
            .canonicalize()
            .map_err(|_| tool_error("checkout root is unavailable"))
    }

    fn resolve_requested_path(&self, path: &Path) -> acp::Result<PathBuf> {
        if path.as_os_str().is_empty() || path.to_string_lossy().contains('\0') {
            return Err(tool_error("path must not be empty"));
        }
        let root = self.checkout_root()?;
        let relative = checkout_relative_path(path)?;
        Ok(root.join(relative))
    }

    fn open_read_file(&self, path: &Path) -> acp::Result<fs::File> {
        let root = self.checkout_root()?;
        let relative = checkout_relative_path(path)?;
        open_checkout_file(&root, &relative, CheckoutFileOpenMode::Read)
    }

    fn open_write_file(&self, path: &Path) -> acp::Result<fs::File> {
        let root = self.checkout_root()?;
        let relative = checkout_relative_path(path)?;
        if relative.components().any(
            |component| matches!(component, Component::Normal(name) if name == OsStr::new(".git")),
        ) {
            return Err(tool_error(".git writes are not allowed"));
        }
        open_checkout_file(&root, &relative, CheckoutFileOpenMode::Write)
    }

    fn resolve_cwd(&self, cwd: Option<&Path>) -> acp::Result<PathBuf> {
        match cwd {
            Some(path) => {
                let resolved = self.resolve_requested_path(path)?;
                let root = self.checkout_root()?;
                let canonical = resolved
                    .canonicalize()
                    .map_err(|_| tool_error("terminal cwd does not exist"))?;
                if !canonical.starts_with(root) || !canonical.is_dir() {
                    return Err(tool_error("terminal cwd must be inside the checkout"));
                }
                Ok(canonical)
            }
            None => self.checkout_root(),
        }
    }

    async fn create_terminal(
        &self,
        args: schema::CreateTerminalRequest,
    ) -> acp::Result<schema::CreateTerminalResponse> {
        let boundary = self
            .terminal_boundary
            .as_ref()
            .ok_or_else(|| tool_error("terminal runtime is unavailable"))?;
        if args.command.is_empty() {
            return Err(tool_error("terminal command is required"));
        }
        if args.env.iter().any(|env| !is_safe_env_name(&env.name)) {
            return Err(tool_error("terminal environment variable name is invalid"));
        }
        let cwd = self.resolve_cwd(args.cwd.as_deref())?;
        let chroot_cwd = self.chroot_path_for_checkout_path(&cwd)?;
        let output_limit = args
            .output_byte_limit
            .map(|limit| (limit as usize).min(MAX_TERMINAL_OUTPUT_BYTE_LIMIT))
            .unwrap_or(DEFAULT_TERMINAL_OUTPUT_BYTE_LIMIT);
        let mut command = Command::new(&args.command);
        command.args(&args.args);
        command.env_clear();
        for env in args.env {
            command.env(env.name, env.value);
        }
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        configure_chroot_terminal_command(&mut command, boundary, &chroot_cwd)?;
        let mut child = command
            .spawn()
            .map_err(|_| tool_error("spawning terminal command failed"))?;
        let child_pid = child.id();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let entry = Arc::new(tokio::sync::Mutex::new(TerminalEntry {
            child: Some(Arc::new(tokio::sync::Mutex::new(child))),
            child_pid,
            output_drains: Vec::new(),
            output: String::new(),
            truncated: false,
            output_limit,
            exit_status: None,
        }));
        let mut output_drains = Vec::new();
        if let Some(stdout) = stdout {
            output_drains.push(spawn_output_drain(stdout, entry.clone()));
        }
        if let Some(stderr) = stderr {
            output_drains.push(spawn_output_drain(stderr, entry.clone()));
        }
        if !output_drains.is_empty() {
            entry.lock().await.output_drains = output_drains;
        }
        let terminal_id = {
            let mut terminals = self.terminals.lock().await;
            let id = format!("term_{}", terminals.next_id);
            terminals.next_id = terminals.next_id.wrapping_add(1);
            terminals.entries.insert(id.clone(), entry);
            id
        };
        Ok(schema::CreateTerminalResponse::new(terminal_id))
    }

    async fn terminal_output(
        &self,
        args: schema::TerminalOutputRequest,
    ) -> acp::Result<schema::TerminalOutputResponse> {
        let entry = self.terminal_entry(&args.terminal_id.to_string()).await?;
        let output_drains = {
            let mut entry = entry.lock().await;
            refresh_exit_status(&mut entry)
        };
        wait_for_output_drains(output_drains).await;
        let entry = entry.lock().await;
        Ok(
            schema::TerminalOutputResponse::new(entry.output.clone(), entry.truncated)
                .exit_status(entry.exit_status.clone()),
        )
    }

    async fn wait_for_terminal_exit(
        &self,
        args: schema::WaitForTerminalExitRequest,
    ) -> acp::Result<schema::WaitForTerminalExitResponse> {
        let entry = self.terminal_entry(&args.terminal_id.to_string()).await?;
        let (cached_status, child, output_drains) = {
            let mut entry = entry.lock().await;
            if let Some(exit_status) = entry.exit_status.clone() {
                let output_drains = std::mem::take(&mut entry.output_drains);
                (Some(exit_status), None, output_drains)
            } else {
                (None, entry.child.clone(), Vec::new())
            }
        };
        if let Some(exit_status) = cached_status {
            wait_for_output_drains(output_drains).await;
            return Ok(schema::WaitForTerminalExitResponse::new(exit_status));
        }
        let Some(child) = child else {
            return Err(tool_error("terminal process is unavailable"));
        };
        let mut child = child.lock().await;
        if let Some(exit_status) = entry.lock().await.exit_status.clone() {
            return Ok(schema::WaitForTerminalExitResponse::new(exit_status));
        }
        let status = child
            .wait()
            .await
            .map(exit_status_from)
            .map_err(|_| tool_error("waiting for terminal failed"))?;
        let (status, output_drains) = {
            let mut entry = entry.lock().await;
            let status = entry.exit_status.clone().unwrap_or_else(|| {
                entry.exit_status = Some(status.clone());
                status
            });
            entry.child = None;
            (status, std::mem::take(&mut entry.output_drains))
        };
        wait_for_output_drains(output_drains).await;
        Ok(schema::WaitForTerminalExitResponse::new(status))
    }

    async fn kill_terminal(
        &self,
        args: schema::KillTerminalRequest,
    ) -> acp::Result<schema::KillTerminalResponse> {
        let entry = self.terminal_entry(&args.terminal_id.to_string()).await?;
        let (child, child_pid) = {
            let entry = entry.lock().await;
            (entry.child.clone(), entry.child_pid)
        };
        signal_terminal_process(child_pid);
        let output_drains = if let Some(child) = child
            && let Ok(mut child) = child.try_lock()
        {
            let status = wait_for_signalled_terminal_child(&mut child)
                .await
                .map(exit_status_from)
                .unwrap_or_else(|_| schema::TerminalExitStatus::new().signal("killed".to_string()));
            let mut entry = entry.lock().await;
            entry.child = None;
            entry.exit_status = Some(status);
            std::mem::take(&mut entry.output_drains)
        } else {
            Vec::new()
        };
        if !output_drains.is_empty() {
            wait_for_output_drains(output_drains).await;
        }
        Ok(schema::KillTerminalResponse::new())
    }

    async fn release_terminal(
        &self,
        args: schema::ReleaseTerminalRequest,
    ) -> acp::Result<schema::ReleaseTerminalResponse> {
        let entry = {
            let mut terminals = self.terminals.lock().await;
            terminals.entries.remove(&args.terminal_id.to_string())
        }
        .ok_or_else(|| tool_error("terminal not found"))?;
        let (child, child_pid, output_drains) = {
            let mut entry = entry.lock().await;
            (
                entry.child.clone(),
                entry.child_pid,
                std::mem::take(&mut entry.output_drains),
            )
        };
        signal_terminal_process(child_pid);
        if let Some(child) = child
            && let Ok(mut child) = child.try_lock()
        {
            let _ = wait_for_signalled_terminal_child(&mut child).await;
        }
        wait_for_output_drains(output_drains).await;
        Ok(schema::ReleaseTerminalResponse::new())
    }

    async fn terminal_entry(
        &self,
        terminal_id: &str,
    ) -> acp::Result<Arc<tokio::sync::Mutex<TerminalEntry>>> {
        self.terminals
            .lock()
            .await
            .entries
            .get(terminal_id)
            .cloned()
            .ok_or_else(|| tool_error("terminal not found"))
    }

    fn sanitize_json_value(&self, value: serde_json::Value) -> serde_json::Value {
        let root = self.sanitize_root();
        sanitize_json_value(value, &root)
    }

    fn sanitize_text(&self, value: &str) -> String {
        sanitize_text(value, &self.sanitize_root())
    }

    fn sanitize_root(&self) -> String {
        self.checkout_root()
            .unwrap_or_else(|_| self.checkout_root.clone())
            .display()
            .to_string()
    }

    fn chroot_path_for_checkout_path(&self, checkout_path: &Path) -> acp::Result<PathBuf> {
        let root = self.checkout_root()?;
        let relative = checkout_path
            .strip_prefix(&root)
            .map_err(|_| tool_error("path is outside the checkout"))?;
        Ok(Path::new(CHROOT_CHECKOUT_ROOT).join(relative))
    }
}

impl Drop for RuntimeToolContext {
    fn drop(&mut self) {
        if let Ok(mut terminals) = self.terminals.try_lock() {
            for entry in terminals.entries.values() {
                if let Ok(entry) = entry.try_lock()
                    && entry.exit_status.is_none()
                {
                    signal_terminal_process(entry.child_pid);
                    if let Some(child) = entry.child.as_ref()
                        && let Ok(mut child) = child.try_lock()
                    {
                        let _ = child.start_kill();
                    }
                }
            }
            terminals.entries.clear();
        }
    }
}

fn checkout_relative_path(path: &Path) -> acp::Result<PathBuf> {
    let mut relative = PathBuf::new();
    let mut components = path.components().peekable();
    if path.is_absolute() {
        match (components.next(), components.next()) {
            (Some(Component::RootDir), Some(Component::Normal(root)))
                if root == OsStr::new(CHROOT_CHECKOUT_ROOT.trim_start_matches('/')) => {}
            _ => return Err(tool_error("absolute paths must be under the checkout root")),
        }
    }
    for component in components {
        match component {
            Component::Normal(name) => relative.push(name),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(tool_error("path traversal is not allowed"));
            }
        }
    }
    Ok(relative)
}

#[derive(Debug, Clone, Copy)]
enum CheckoutFileOpenMode {
    Read,
    Write,
}

impl CheckoutFileOpenMode {
    fn creates_parent_dirs(self) -> bool {
        matches!(self, Self::Write)
    }

    fn open_error(self) -> &'static str {
        match self {
            Self::Read => "reading file failed",
            Self::Write => "opening file for write failed",
        }
    }

    fn non_regular_error(self) -> &'static str {
        match self {
            Self::Read => "path is not a regular file",
            Self::Write => "write target must be a regular file",
        }
    }
}

#[cfg(unix)]
fn open_checkout_file(
    root: &Path,
    relative: &Path,
    mode: CheckoutFileOpenMode,
) -> acp::Result<fs::File> {
    let components = relative
        .components()
        .map(normal_component_name)
        .collect::<acp::Result<Vec<_>>>()?;
    let Some((file_name, parent_names)) = components.split_last() else {
        return Err(tool_error("file path must name a file"));
    };
    let mut dir = open_root_dir(root)?;
    for parent_name in parent_names {
        dir = open_checkout_dir_component(&dir, parent_name, mode.creates_parent_dirs())?;
    }
    let file = open_checkout_file_component(&dir, file_name, mode)?;
    validate_opened_regular_file(&file, mode)?;
    Ok(file)
}

#[cfg(not(unix))]
fn open_checkout_file(
    _root: &Path,
    _relative: &Path,
    _mode: CheckoutFileOpenMode,
) -> acp::Result<fs::File> {
    Err(tool_error(
        "checkout filesystem tools are not supported on this platform",
    ))
}

fn normal_component_name(component: Component<'_>) -> acp::Result<&OsStr> {
    let Component::Normal(name) = component else {
        return Err(tool_error("path traversal is not allowed"));
    };
    Ok(name)
}

#[cfg(unix)]
fn open_root_dir(root: &Path) -> acp::Result<OwnedFd> {
    let path = cstring_path(root)?;
    let fd = unsafe {
        libc::open(
            path.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    };
    owned_fd_from_raw(fd, "checkout root is unavailable")
}

#[cfg(unix)]
fn open_checkout_dir_component(
    dir: &OwnedFd,
    name: &OsStr,
    create_missing: bool,
) -> acp::Result<OwnedFd> {
    let name = cstring_component(name)?;
    match open_dir_component(dir, &name) {
        Ok(child) => Ok(child),
        Err(error) if create_missing && error.raw_os_error() == Some(libc::ENOENT) => {
            create_dir_component(dir, &name)?;
            open_dir_component(dir, &name)
                .map_err(|_| tool_error("parent directory is unavailable"))
        }
        Err(_) => Err(tool_error("parent path must be a real directory")),
    }
}

#[cfg(unix)]
fn open_dir_component(dir: &OwnedFd, name: &CString) -> std::io::Result<OwnedFd> {
    let fd = unsafe {
        libc::openat(
            dir.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }
}

#[cfg(unix)]
fn create_dir_component(dir: &OwnedFd, name: &CString) -> acp::Result<()> {
    if unsafe { libc::mkdirat(dir.as_raw_fd(), name.as_ptr(), 0o777) } == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::EEXIST) {
        Ok(())
    } else {
        Err(tool_error("creating parent directories failed"))
    }
}

#[cfg(unix)]
fn open_checkout_file_component(
    dir: &OwnedFd,
    name: &OsStr,
    mode: CheckoutFileOpenMode,
) -> acp::Result<fs::File> {
    let name = cstring_component(name)?;
    let access_flags = match mode {
        CheckoutFileOpenMode::Read => libc::O_RDONLY,
        CheckoutFileOpenMode::Write => libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC,
    };
    let fd = unsafe {
        libc::openat(
            dir.as_raw_fd(),
            name.as_ptr(),
            access_flags | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
            0o666,
        )
    };
    if fd < 0 {
        return Err(tool_error(mode.open_error()));
    }
    Ok(unsafe { fs::File::from_raw_fd(fd) })
}

#[cfg(unix)]
fn validate_opened_regular_file(file: &fs::File, mode: CheckoutFileOpenMode) -> acp::Result<()> {
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe { libc::fstat(file.as_raw_fd(), stat.as_mut_ptr()) } != 0 {
        return Err(tool_error(mode.open_error()));
    }
    let stat = unsafe { stat.assume_init() };
    if (stat.st_mode & libc::S_IFMT) != libc::S_IFREG {
        return Err(tool_error(mode.non_regular_error()));
    }
    Ok(())
}

#[cfg(unix)]
fn cstring_path(path: &Path) -> acp::Result<CString> {
    CString::new(path.as_os_str().as_bytes()).map_err(|_| tool_error("path contains a NUL byte"))
}

#[cfg(unix)]
fn cstring_component(name: &OsStr) -> acp::Result<CString> {
    CString::new(name.as_bytes()).map_err(|_| tool_error("path contains a NUL byte"))
}

#[cfg(unix)]
fn owned_fd_from_raw(fd: libc::c_int, error: &'static str) -> acp::Result<OwnedFd> {
    if fd < 0 {
        Err(tool_error(error))
    } else {
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }
}

fn terminal_boundary_for_checkout(checkout_root: &Path) -> Option<TerminalBoundary> {
    if !is_chroot_checkout_root(checkout_root) {
        return None;
    }
    Some(TerminalBoundary {
        root_dir: checkout_root.parent()?.to_path_buf(),
        run_uid: DEFAULT_AGENT_RUN_UID,
        run_gid: DEFAULT_AGENT_RUN_GID,
    })
}

fn is_chroot_checkout_root(checkout_root: &Path) -> bool {
    let mut components = checkout_root.components().rev();
    matches!(
        (
            components.next(),
            components.next(),
            components.next(),
            components.next()
        ),
        (
            Some(Component::Normal(workspace)),
            Some(Component::Normal(root)),
            Some(Component::Normal(_session_id)),
            Some(Component::Normal(agent_runtimes)),
        ) if workspace == OsStr::new(CHROOT_CHECKOUT_ROOT.trim_start_matches('/'))
            && root == OsStr::new("root")
            && agent_runtimes == OsStr::new(AGENT_RUNTIMES_DIR_NAME)
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn configure_chroot_terminal_command(
    command: &mut Command,
    boundary: &TerminalBoundary,
    cwd: &Path,
) -> acp::Result<()> {
    use std::os::unix::ffi::OsStrExt;

    #[cfg(test)]
    if boundary
        .root_dir
        .join(TEST_SKIP_CHROOT_PREEXEC_MARKER)
        .exists()
    {
        if let Ok(relative_cwd) = cwd.strip_prefix(Path::new("/")) {
            command.current_dir(boundary.root_dir.join(relative_cwd));
        }
        return Ok(());
    }

    let root = std::ffi::CString::new(boundary.root_dir.as_os_str().as_bytes())
        .map_err(|_| tool_error("terminal chroot root is invalid"))?;
    let cwd = std::ffi::CString::new(cwd.as_os_str().as_bytes())
        .map_err(|_| tool_error("terminal cwd is invalid"))?;
    let run_uid = boundary.run_uid;
    let run_gid = boundary.run_gid;
    unsafe {
        command.pre_exec(move || prepare_chroot_terminal_child(&root, &cwd, run_uid, run_gid));
    }
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn configure_chroot_terminal_command(
    _command: &mut Command,
    _boundary: &TerminalBoundary,
    _cwd: &Path,
) -> acp::Result<()> {
    Err(tool_error(
        "chroot terminal is not supported on this platform",
    ))
}

#[cfg(target_os = "linux")]
fn prepare_chroot_terminal_child(
    root: &std::ffi::CString,
    cwd: &std::ffi::CString,
    run_uid: u32,
    run_gid: u32,
) -> std::io::Result<()> {
    unsafe {
        check_nonnegative_syscall(libc::setsid())?;
        check_zero_syscall(libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0))?;
        check_zero_syscall(libc::chroot(root.as_ptr()))?;
        check_zero_syscall(libc::chdir(cwd.as_ptr()))?;
        check_zero_syscall(libc::setgroups(0, std::ptr::null()))?;
        check_zero_syscall(libc::setgid(run_gid))?;
        check_zero_syscall(libc::setuid(run_uid))?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn prepare_chroot_terminal_child(
    root: &std::ffi::CString,
    cwd: &std::ffi::CString,
    run_uid: u32,
    run_gid: u32,
) -> std::io::Result<()> {
    unsafe {
        check_nonnegative_syscall(libc::setsid())?;
        check_zero_syscall(libc::chroot(root.as_ptr()))?;
        check_zero_syscall(libc::chdir(cwd.as_ptr()))?;
        check_zero_syscall(libc::setgroups(0, std::ptr::null()))?;
        check_zero_syscall(libc::setgid(run_gid))?;
        check_zero_syscall(libc::setuid(run_uid))?;
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn check_nonnegative_syscall(result: libc::c_int) -> std::io::Result<()> {
    if result >= 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn check_zero_syscall(result: libc::c_int) -> std::io::Result<()> {
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

fn spawn_output_drain(
    mut reader: impl tokio::io::AsyncRead + Send + Unpin + 'static,
    entry: Arc<tokio::sync::Mutex<TerminalEntry>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut buffer = [0_u8; 4096];
        loop {
            let read = match reader.read(&mut buffer).await {
                Ok(0) | Err(_) => return,
                Ok(read) => read,
            };
            let text = String::from_utf8_lossy(&buffer[..read]);
            let mut entry = entry.lock().await;
            append_bounded_output(&mut entry, &text);
        }
    })
}

fn append_bounded_output(entry: &mut TerminalEntry, text: &str) {
    entry.output.push_str(text);
    if entry.output.len() <= entry.output_limit {
        return;
    }
    entry.truncated = true;
    let trim_to = entry.output.len() - entry.output_limit;
    let boundary = entry
        .output
        .char_indices()
        .map(|(idx, _)| idx)
        .find(|idx| *idx >= trim_to)
        .unwrap_or(entry.output.len());
    entry.output.drain(..boundary);
}

fn signal_terminal_process(pid: Option<u32>) {
    #[cfg(unix)]
    if let Some(pid) = pid.and_then(|id| i32::try_from(id).ok()) {
        unsafe {
            let _ = libc::kill(-pid, libc::SIGKILL);
            let _ = libc::kill(pid, libc::SIGKILL);
        }
    }
}

async fn wait_for_signalled_terminal_child(
    child: &mut Child,
) -> std::io::Result<std::process::ExitStatus> {
    let _ = child.start_kill();
    child.wait().await
}

async fn wait_for_output_drains(output_drains: Vec<tokio::task::JoinHandle<()>>) {
    for output_drain in output_drains {
        let _ = output_drain.await;
    }
}

fn refresh_exit_status(entry: &mut TerminalEntry) -> Vec<tokio::task::JoinHandle<()>> {
    if entry.exit_status.is_some() {
        return std::mem::take(&mut entry.output_drains);
    }
    let Some(child) = entry.child.clone() else {
        return Vec::new();
    };
    let Ok(mut child) = child.try_lock() else {
        return Vec::new();
    };
    if let Ok(Some(status)) = child.try_wait() {
        entry.exit_status = Some(exit_status_from(status));
        entry.child = None;
        return std::mem::take(&mut entry.output_drains);
    }
    Vec::new()
}

fn exit_status_from(status: std::process::ExitStatus) -> schema::TerminalExitStatus {
    let mut exit_status = schema::TerminalExitStatus::new();
    if let Some(code) = status.code()
        && let Ok(code) = u32::try_from(code)
    {
        exit_status = exit_status.exit_code(code);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            exit_status = exit_status.signal(signal.to_string());
        }
    }
    exit_status
}

fn is_safe_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn sanitize_json_value(value: serde_json::Value, host_root: &str) -> serde_json::Value {
    match value {
        serde_json::Value::String(value) => {
            serde_json::Value::String(sanitize_text(&value, host_root))
        }
        serde_json::Value::Array(values) => serde_json::Value::Array(
            values
                .into_iter()
                .map(|value| sanitize_json_value(value, host_root))
                .collect(),
        ),
        serde_json::Value::Object(values) => serde_json::Value::Object(
            values
                .into_iter()
                .map(|(key, value)| {
                    (
                        sanitize_text(&key, host_root),
                        sanitize_json_value(value, host_root),
                    )
                })
                .collect(),
        ),
        value => value,
    }
}

fn sanitize_text(value: &str, host_root: &str) -> String {
    value.replace(host_root, CHROOT_CHECKOUT_ROOT)
}

fn tool_kind_name(kind: schema::ToolKind) -> &'static str {
    match kind {
        schema::ToolKind::Read => "read",
        schema::ToolKind::Edit => "edit",
        schema::ToolKind::Delete => "delete",
        schema::ToolKind::Move => "move",
        schema::ToolKind::Search => "search",
        schema::ToolKind::Execute => "execute",
        schema::ToolKind::Think => "think",
        schema::ToolKind::Fetch => "fetch",
        schema::ToolKind::SwitchMode => "switch_mode",
        schema::ToolKind::Other => "other",
        _ => "other",
    }
}

fn tool_status_name(status: schema::ToolCallStatus) -> &'static str {
    match status {
        schema::ToolCallStatus::Pending => "pending",
        schema::ToolCallStatus::InProgress => "in_progress",
        schema::ToolCallStatus::Completed => "completed",
        schema::ToolCallStatus::Failed => "failed",
        _ => "pending",
    }
}

fn tool_error(message: &'static str) -> acp::Error {
    acp::Error::invalid_params().data(serde_json::json!(message))
}

fn prompt_response_notification_deadline() -> tokio::time::Instant {
    tokio::time::Instant::now() + PROMPT_RESPONSE_NOTIFICATION_GRACE
}

pub(super) fn content_text(content: schema::ContentBlock) -> String {
    match content {
        schema::ContentBlock::Text(text) => text.text,
        schema::ContentBlock::Image(_) => "<image>".to_string(),
        schema::ContentBlock::Audio(_) => "<audio>".to_string(),
        schema::ContentBlock::ResourceLink(link) => link.uri,
        content => resource_placeholder(matches!(content, schema::ContentBlock::Resource(_))),
    }
}

fn resource_placeholder(is_resource: bool) -> String {
    ["<unsupported>", "<resource>"][usize::from(is_resource)].to_string()
}

pub(super) fn permission_option_ids(
    args: &schema::RequestPermissionRequest,
) -> Result<(String, String), MockClientError> {
    if args.options.iter().any(|option| {
        matches!(
            option.kind,
            schema::PermissionOptionKind::AllowAlways | schema::PermissionOptionKind::RejectAlways
        )
    }) {
        return UnsupportedPermissionOptionsSnafu.fail();
    }

    let approve_option_id = unique_option_id(args, schema::PermissionOptionKind::AllowOnce)?;
    let deny_option_id = unique_option_id(args, schema::PermissionOptionKind::RejectOnce)?;

    match (approve_option_id, deny_option_id) {
        (Some(approve_option_id), Some(deny_option_id)) => Ok((approve_option_id, deny_option_id)),
        _ => InvalidPermissionOptionsSnafu.fail(),
    }
}

fn unique_option_id(
    args: &schema::RequestPermissionRequest,
    kind: schema::PermissionOptionKind,
) -> Result<Option<String>, MockClientError> {
    let mut matches = args
        .options
        .iter()
        .filter(|option| option.kind == kind)
        .map(|option| option.option_id.to_string());
    let first = matches.next();
    if matches.next().is_some() {
        return UnsupportedPermissionOptionsSnafu.fail();
    }
    Ok(first)
}

fn to_acp_error(source: crate::sessions::SessionStoreError) -> acp::Error {
    let _ = source;
    acp::Error::internal_error()
}
