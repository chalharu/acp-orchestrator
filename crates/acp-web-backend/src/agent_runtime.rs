use std::{
    collections::HashMap,
    env,
    fmt::Display,
    fs,
    net::{SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Condvar, Mutex},
    time::Duration,
};

#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::ffi::CString;

use crate::workspace_checkout::PreparedWorkspaceCheckout;

pub const AGENT_RUNTIMES_DIR_NAME: &str = crate::workspace_checkout::AGENT_RUNTIMES_DIR_NAME;
pub const CHROOT_CHECKOUT_ROOT: &str = "/workspace";
pub const DEFAULT_AGENT_RUN_UID: u32 = 65_534;
pub const DEFAULT_AGENT_RUN_GID: u32 = 65_534;
pub const DEFAULT_AGENT_LAUNCH_TIMEOUT: Duration = Duration::from_secs(30);
#[cfg(target_os = "linux")]
const DEFAULT_AGENT_CGROUP_ROOT: &str = "/sys/fs/cgroup/acp-orchestrator";
const ACP_HOST: &str = "127.0.0.1";
const ACP_PORT_PLACEHOLDER: &str = "${ACP_PORT}";
const ACP_ENDPOINT_PLACEHOLDER: &str = "${ACP_ENDPOINT}";
const ACP_BASE_URL_PLACEHOLDER: &str = "${ACP_BASE_URL}";
const ACP_HOST_PLACEHOLDER: &str = "${ACP_HOST}";
#[cfg(test)]
const TEST_FAKE_CGROUP_MARKER: &str = ".acp-test-fake-cgroup-v2";
#[cfg(test)]
const TEST_SKIP_CHROOT_PREEXEC_MARKER: &str = ".acp-test-skip-chroot-preexec";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentLaunchMode {
    Chroot,
}

impl AgentLaunchMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Chroot => "chroot",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentLaunchConfig {
    pub mode: AgentLaunchMode,
    pub command: Vec<String>,
    pub env_allowlist: Vec<String>,
    pub timeout: Duration,
    pub run_uid: u32,
    pub run_gid: u32,
}

impl AgentLaunchConfig {
    pub fn chroot(
        command: Vec<String>,
        env_allowlist: Vec<String>,
        timeout: Duration,
        run_uid: u32,
        run_gid: u32,
    ) -> Result<Self, AgentLaunchConfigError> {
        let config = Self {
            mode: AgentLaunchMode::Chroot,
            command,
            env_allowlist,
            timeout,
            run_uid,
            run_gid,
        };
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), AgentLaunchConfigError> {
        if self.command.is_empty() {
            return Err(AgentLaunchConfigError::MissingCommand);
        }
        if self.command.iter().any(|arg| arg.is_empty()) {
            return Err(AgentLaunchConfigError::EmptyArgvElement);
        }
        if self.timeout.is_zero() {
            return Err(AgentLaunchConfigError::InvalidTimeout);
        }
        if self.run_uid == 0 {
            return Err(AgentLaunchConfigError::RootRunUid);
        }
        if self.run_gid == 0 {
            return Err(AgentLaunchConfigError::RootRunGid);
        }
        if let Some(name) = self
            .env_allowlist
            .iter()
            .find(|name| !Self::is_safe_env_name(name))
        {
            return Err(AgentLaunchConfigError::InvalidEnvName(name.clone()));
        }
        Ok(())
    }

    fn is_safe_env_name(name: &str) -> bool {
        let mut chars = name.chars();
        let Some(first) = chars.next() else {
            return false;
        };
        (first == '_' || first.is_ascii_alphabetic())
            && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentLaunchConfigError {
    MissingCommand,
    EmptyArgvElement,
    InvalidTimeout,
    RootRunUid,
    RootRunGid,
    InvalidEnvName(String),
}

impl Display for AgentLaunchConfigError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingCommand => formatter.write_str("agent command is required in chroot mode"),
            Self::EmptyArgvElement => {
                formatter.write_str("agent command argv elements must not be empty")
            }
            Self::InvalidTimeout => {
                formatter.write_str("agent launch timeout must be greater than zero")
            }
            Self::RootRunUid => formatter.write_str("agent run uid must be non-root"),
            Self::RootRunGid => formatter.write_str("agent run gid must be non-root"),
            Self::InvalidEnvName(name) => {
                write!(formatter, "agent env allowlist name is invalid: {name}")
            }
        }
    }
}

impl std::error::Error for AgentLaunchConfigError {}

#[derive(Debug)]
pub enum AgentRuntimeError {
    Config(AgentLaunchConfigError),
    Io(String),
    Unsupported(String),
    AlreadyRunning(String),
    LaunchTimedOut(Duration),
    Poisoned(String),
}

impl Display for AgentRuntimeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Config(error) => Display::fmt(error, formatter),
            Self::Io(message) | Self::Unsupported(message) | Self::Poisoned(message) => {
                formatter.write_str(message)
            }
            Self::AlreadyRunning(session_id) => {
                write!(
                    formatter,
                    "agent runtime already exists for session {session_id}"
                )
            }
            Self::LaunchTimedOut(timeout) => {
                write!(formatter, "agent launch timed out after {timeout:?}")
            }
        }
    }
}

impl std::error::Error for AgentRuntimeError {}

impl From<AgentLaunchConfigError> for AgentRuntimeError {
    fn from(error: AgentLaunchConfigError) -> Self {
        Self::Config(error)
    }
}

#[derive(Debug)]
pub struct AgentSessionLaunch<'a> {
    pub session_id: &'a str,
    pub workspace_id: &'a str,
    pub checkout: &'a PreparedWorkspaceCheckout,
    pub config: Option<AgentLaunchConfig>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentLaunchMetadata {
    pub acp_address: Option<String>,
}

pub trait AgentRuntimeManager: Send + Sync + std::fmt::Debug {
    fn launch_session(
        &self,
        launch: &AgentSessionLaunch<'_>,
    ) -> Result<AgentLaunchMetadata, AgentRuntimeError>;
    fn forget_session(&self, session_id: &str);
}

pub type DynAgentRuntimeManager = Arc<dyn AgentRuntimeManager>;

pub async fn launch_session_blocking(
    runtime_manager: DynAgentRuntimeManager,
    session_id: String,
    workspace_id: String,
    checkout: PreparedWorkspaceCheckout,
    config: Option<AgentLaunchConfig>,
) -> Result<AgentLaunchMetadata, AgentRuntimeError> {
    match tokio::task::spawn_blocking(move || {
        runtime_manager.launch_session(&AgentSessionLaunch {
            session_id: &session_id,
            workspace_id: &workspace_id,
            checkout: &checkout,
            config,
        })
    })
    .await
    {
        Ok(result) => result,
        Err(error) => Err(AgentRuntimeError::Io(format!(
            "joining agent runtime launch failed: {error}"
        ))),
    }
}

#[derive(Debug, Default)]
pub struct NoopAgentRuntimeManager;

impl AgentRuntimeManager for NoopAgentRuntimeManager {
    fn launch_session(
        &self,
        _launch: &AgentSessionLaunch<'_>,
    ) -> Result<AgentLaunchMetadata, AgentRuntimeError> {
        Ok(AgentLaunchMetadata::default())
    }

    fn forget_session(&self, _session_id: &str) {}
}

#[derive(Debug)]
pub struct FsAgentRuntimeManager {
    state_dir: PathBuf,
    config: Option<AgentLaunchConfig>,
    children: Mutex<AgentChildRegistry>,
    launch_notifications: Condvar,
    cgroup_root: Option<PathBuf>,
}

#[derive(Debug, Default)]
struct AgentChildRegistry {
    slots: HashMap<String, AgentChildSlot>,
    next_launch_id: u64,
}

#[derive(Debug)]
enum AgentChildSlot {
    Launching(u64),
    Running(AgentChild),
}

#[derive(Debug)]
struct AgentChild {
    child: Child,
    cgroup: Option<AgentCgroup>,
    metadata: AgentLaunchMetadata,
}

struct PreparedChrootLaunch {
    command: Command,
    cgroup: AgentCgroup,
    acp_endpoint: Option<AcpEndpoint>,
    port_reservation: Option<TcpListener>,
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone)]
struct AgentCgroup {
    path: PathBuf,
}

#[cfg(not(target_os = "linux"))]
#[derive(Debug, Clone)]
struct AgentCgroup;

impl FsAgentRuntimeManager {
    pub fn new(
        state_dir: PathBuf,
        config: Option<AgentLaunchConfig>,
    ) -> Result<Self, AgentRuntimeError> {
        Self::new_with_cgroup_root(state_dir, config, default_agent_cgroup_root())
    }

    fn new_with_cgroup_root(
        state_dir: PathBuf,
        config: Option<AgentLaunchConfig>,
        cgroup_root: Option<PathBuf>,
    ) -> Result<Self, AgentRuntimeError> {
        if let Some(config) = config.as_ref() {
            config.validate()?;
        }
        Ok(Self {
            state_dir,
            config,
            children: Mutex::new(AgentChildRegistry::default()),
            launch_notifications: Condvar::new(),
            cgroup_root,
        })
    }

    pub fn runtime_dir_for_state(state_dir: &Path, session_id: &str) -> PathBuf {
        state_dir.join(AGENT_RUNTIMES_DIR_NAME).join(session_id)
    }

    pub fn root_dir_for_state(state_dir: &Path, session_id: &str) -> PathBuf {
        Self::runtime_dir_for_state(state_dir, session_id).join("root")
    }

    fn runtime_dir(&self, session_id: &str) -> PathBuf {
        Self::runtime_dir_for_state(&self.state_dir, session_id)
    }

    fn root_dir(&self, session_id: &str) -> PathBuf {
        Self::root_dir_for_state(&self.state_dir, session_id)
    }

    fn launch_chroot(
        &self,
        config: &AgentLaunchConfig,
        launch: &AgentSessionLaunch<'_>,
    ) -> Result<(AgentChild, AgentLaunchMetadata), AgentRuntimeError> {
        let PreparedChrootLaunch {
            command,
            cgroup,
            acp_endpoint,
            port_reservation,
        } = self.prepare_chroot_launch(config, launch)?;
        let mut child =
            spawn_with_timeout(command, config.timeout, Some(cgroup), port_reservation)?;
        if let Err(error) =
            wait_for_acp_readiness(acp_endpoint.as_ref(), config.timeout, &mut child)
        {
            terminate_agent_child(&mut child);
            return Err(error);
        }
        let metadata = AgentLaunchMetadata {
            acp_address: acp_endpoint.map(|endpoint| endpoint.address),
        };
        child.metadata = metadata.clone();
        Ok((child, metadata))
    }

    fn prepare_chroot_launch(
        &self,
        config: &AgentLaunchConfig,
        launch: &AgentSessionLaunch<'_>,
    ) -> Result<PreparedChrootLaunch, AgentRuntimeError> {
        let root_dir = self.root_dir(launch.session_id);
        fs::create_dir_all(&root_dir).map_err(|error| {
            AgentRuntimeError::Io(format!("creating agent chroot root failed: {error}"))
        })?;
        chown_workspace_for_agent(&launch.checkout.working_dir, config.run_uid, config.run_gid)?;
        let cgroup = self.prepare_session_cgroup(launch.session_id)?;
        let reserved_endpoint = AcpEndpoint::reserve_for_command(&config.command)?;
        let acp_endpoint = reserved_endpoint
            .as_ref()
            .map(|reserved| reserved.endpoint.clone());
        let mut command = build_agent_command(config, launch, acp_endpoint.as_ref());
        configure_chroot_command(
            &mut command,
            &root_dir,
            &cgroup,
            config.run_uid,
            config.run_gid,
        )?;
        Ok(PreparedChrootLaunch {
            command,
            cgroup,
            acp_endpoint,
            port_reservation: reserved_endpoint.map(ReservedAcpEndpoint::into_listener),
        })
    }

    fn prepare_session_cgroup(&self, session_id: &str) -> Result<AgentCgroup, AgentRuntimeError> {
        #[cfg(target_os = "macos")]
        {
            AgentCgroup::prepare(Path::new(""), session_id)
        }
        #[cfg(target_os = "linux")]
        {
            let Some(root) = self.cgroup_root.as_ref() else {
                return Err(AgentRuntimeError::Unsupported(
                    "chroot agent launch requires a Linux cgroup v2 root".to_string(),
                ));
            };
            AgentCgroup::prepare(root, session_id)
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            AgentCgroup::prepare(Path::new(""), session_id)
        }
    }

    fn reserve_launch(&self, session_id: &str) -> Result<u64, AgentRuntimeError> {
        let mut registry = self.children.lock().map_err(|_| {
            AgentRuntimeError::Poisoned("agent runtime child registry is poisoned".to_string())
        })?;
        loop {
            match registry.slots.get(session_id) {
                None => {
                    let launch_id = registry.next_launch_id;
                    registry.next_launch_id = registry.next_launch_id.wrapping_add(1);
                    registry
                        .slots
                        .insert(session_id.to_string(), AgentChildSlot::Launching(launch_id));
                    return Ok(launch_id);
                }
                Some(AgentChildSlot::Launching(_)) => {
                    registry = self.launch_notifications.wait(registry).map_err(|_| {
                        AgentRuntimeError::Poisoned(
                            "agent runtime child registry is poisoned".to_string(),
                        )
                    })?;
                }
                Some(AgentChildSlot::Running(_)) => {
                    return Err(AgentRuntimeError::AlreadyRunning(session_id.to_string()));
                }
            }
        }
    }

    fn clear_launch_reservation(&self, session_id: &str, launch_id: u64) {
        let Ok(mut registry) = self.children.lock() else {
            return;
        };
        if matches!(
            registry.slots.get(session_id),
            Some(AgentChildSlot::Launching(existing_id)) if *existing_id == launch_id
        ) {
            registry.slots.remove(session_id);
            self.launch_notifications.notify_all();
        }
    }

    fn store_launched_child(
        &self,
        session_id: &str,
        launch_id: u64,
        mut child: AgentChild,
    ) -> Result<(), AgentRuntimeError> {
        let mut registry = self.children.lock().map_err(|_| {
            terminate_agent_child(&mut child);
            AgentRuntimeError::Poisoned("agent runtime child registry is poisoned".to_string())
        })?;
        let Some(slot) = registry.slots.get_mut(session_id) else {
            terminate_agent_child(&mut child);
            return Err(AgentRuntimeError::Io(
                "agent runtime launch was cancelled".to_string(),
            ));
        };
        if matches!(slot, AgentChildSlot::Launching(existing_id) if *existing_id == launch_id) {
            *slot = AgentChildSlot::Running(child);
            self.launch_notifications.notify_all();
            return Ok(());
        }
        match slot {
            AgentChildSlot::Launching(_) => {
                terminate_agent_child(&mut child);
                Err(AgentRuntimeError::Io(
                    "agent runtime launch was cancelled".to_string(),
                ))
            }
            AgentChildSlot::Running(_) => {
                terminate_agent_child(&mut child);
                Err(AgentRuntimeError::AlreadyRunning(session_id.to_string()))
            }
        }
    }

    fn running_launch_metadata(
        &self,
        session_id: &str,
    ) -> Result<AgentLaunchMetadata, AgentRuntimeError> {
        let registry = self.children.lock().map_err(|_| {
            AgentRuntimeError::Poisoned("agent runtime child registry is poisoned".to_string())
        })?;
        match registry.slots.get(session_id) {
            Some(AgentChildSlot::Running(child)) => Ok(child.metadata.clone()),
            _ => Err(AgentRuntimeError::AlreadyRunning(session_id.to_string())),
        }
    }
}

impl AgentRuntimeManager for FsAgentRuntimeManager {
    fn launch_session(
        &self,
        launch: &AgentSessionLaunch<'_>,
    ) -> Result<AgentLaunchMetadata, AgentRuntimeError> {
        let config = match launch.config.as_ref().or(self.config.as_ref()) {
            Some(config) => config,
            None => return Ok(AgentLaunchMetadata::default()),
        };
        config.validate()?;
        let launch_id = match self.reserve_launch(launch.session_id) {
            Ok(launch_id) => launch_id,
            Err(AgentRuntimeError::AlreadyRunning(_)) => {
                return self.running_launch_metadata(launch.session_id);
            }
            Err(error) => return Err(error),
        };
        let launched = match config.mode {
            AgentLaunchMode::Chroot => self.launch_chroot(config, launch),
        };
        let (child, metadata) = match launched {
            Ok(launched) => launched,
            Err(error) => {
                self.clear_launch_reservation(launch.session_id, launch_id);
                return Err(error);
            }
        };
        self.store_launched_child(launch.session_id, launch_id, child)?;
        Ok(metadata)
    }

    fn forget_session(&self, session_id: &str) {
        let slot = self.children.lock().ok().and_then(|mut registry| {
            let slot = registry.slots.remove(session_id);
            if slot.is_some() {
                self.launch_notifications.notify_all();
            }
            slot
        });
        if let Some(AgentChildSlot::Running(mut child)) = slot {
            terminate_agent_child(&mut child);
        }
        let runtime_dir = self.runtime_dir(session_id);
        if let Err(error) = fs::remove_dir_all(&runtime_dir)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(
                session_id,
                path = %runtime_dir.display(),
                "failed to remove agent runtime directory: {error}"
            );
        }
    }
}

impl Drop for FsAgentRuntimeManager {
    fn drop(&mut self) {
        let Ok(registry) = self.children.get_mut() else {
            return;
        };
        for (_, slot) in registry.slots.drain() {
            let AgentChildSlot::Running(mut child) = slot else {
                continue;
            };
            terminate_agent_child(&mut child);
        }
    }
}

#[cfg(target_os = "linux")]
fn default_agent_cgroup_root() -> Option<PathBuf> {
    Some(PathBuf::from(DEFAULT_AGENT_CGROUP_ROOT))
}

#[cfg(not(target_os = "linux"))]
fn default_agent_cgroup_root() -> Option<PathBuf> {
    None
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AcpEndpoint {
    address: String,
    port: u16,
}

impl AcpEndpoint {
    fn reserve_for_command(
        command: &[String],
    ) -> Result<Option<ReservedAcpEndpoint>, AgentRuntimeError> {
        if !command.iter().any(|arg| contains_acp_placeholder(arg)) {
            return Ok(None);
        }
        let listener = TcpListener::bind((ACP_HOST, 0)).map_err(|error| {
            AgentRuntimeError::Io(format!("allocating ACP listen port failed: {error}"))
        })?;
        let port = listener
            .local_addr()
            .map_err(|error| {
                AgentRuntimeError::Io(format!("reading ACP listen port failed: {error}"))
            })?
            .port();
        Ok(Some(ReservedAcpEndpoint {
            endpoint: Self {
                address: format!("{ACP_HOST}:{port}"),
                port,
            },
            listener,
        }))
    }

    fn base_url(&self) -> String {
        format!("http://{}", self.address)
    }
}

#[derive(Debug)]
struct ReservedAcpEndpoint {
    endpoint: AcpEndpoint,
    listener: TcpListener,
}

impl ReservedAcpEndpoint {
    fn into_listener(self) -> TcpListener {
        self.listener
    }
}

fn contains_acp_placeholder(arg: &str) -> bool {
    [
        ACP_PORT_PLACEHOLDER,
        ACP_ENDPOINT_PLACEHOLDER,
        ACP_BASE_URL_PLACEHOLDER,
        ACP_HOST_PLACEHOLDER,
    ]
    .iter()
    .any(|placeholder| arg.contains(placeholder))
}

fn expand_agent_argv(command: &[String], endpoint: Option<&AcpEndpoint>) -> Vec<String> {
    command
        .iter()
        .map(|arg| expand_agent_arg(arg, endpoint))
        .collect()
}

fn build_agent_command(
    config: &AgentLaunchConfig,
    launch: &AgentSessionLaunch<'_>,
    endpoint: Option<&AcpEndpoint>,
) -> Command {
    let argv = expand_agent_argv(&config.command, endpoint);
    let mut command = Command::new(&argv[0]);
    command.args(&argv[1..]);
    command.env_clear();
    for name in &config.env_allowlist {
        if let Some(value) = env::var_os(name) {
            command.env(name, value);
        }
    }
    apply_structured_agent_env(&mut command, launch, endpoint);
    command.stdin(Stdio::null());
    command.stdout(Stdio::null());
    command.stderr(Stdio::null());
    command
}

fn expand_agent_arg(arg: &str, endpoint: Option<&AcpEndpoint>) -> String {
    let Some(endpoint) = endpoint else {
        return arg.to_string();
    };
    arg.replace(ACP_PORT_PLACEHOLDER, &endpoint.port.to_string())
        .replace(ACP_ENDPOINT_PLACEHOLDER, &endpoint.address)
        .replace(ACP_BASE_URL_PLACEHOLDER, &endpoint.base_url())
        .replace(ACP_HOST_PLACEHOLDER, ACP_HOST)
}

fn apply_structured_agent_env(
    command: &mut Command,
    launch: &AgentSessionLaunch<'_>,
    endpoint: Option<&AcpEndpoint>,
) {
    command.env("ACP_SESSION_ID", launch.session_id);
    command.env("ACP_WORKSPACE_ID", launch.workspace_id);
    command.env("ACP_CHECKOUT_ROOT", CHROOT_CHECKOUT_ROOT);
    command.env("ACP_CHECKOUT_RELPATH", "workspace");
    command.env("ACP_AGENT_LAUNCH_MODE", AgentLaunchMode::Chroot.as_str());
    if let Some(endpoint) = endpoint {
        command.env("ACP_HOST", ACP_HOST);
        command.env("ACP_PORT", endpoint.port.to_string());
        command.env("ACP_ENDPOINT", &endpoint.address);
        command.env("ACP_BASE_URL", endpoint.base_url());
    }
}

fn wait_for_acp_readiness(
    endpoint: Option<&AcpEndpoint>,
    timeout: Duration,
    child: &mut AgentChild,
) -> Result<(), AgentRuntimeError> {
    let Some(endpoint) = endpoint else {
        return Ok(());
    };
    let address: SocketAddr = endpoint.address.parse().map_err(|error| {
        AgentRuntimeError::Io(format!("parsing ACP endpoint address failed: {error}"))
    })?;
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if TcpStream::connect_timeout(&address, Duration::from_millis(100)).is_ok() {
            return Ok(());
        }
        if let Some(status) = child.child.try_wait().map_err(|error| {
            AgentRuntimeError::Io(format!("checking agent process status failed: {error}"))
        })? {
            return Err(AgentRuntimeError::Io(format!(
                "agent process exited before ACP endpoint became ready: {status}"
            )));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    Err(AgentRuntimeError::LaunchTimedOut(timeout))
}

#[cfg(target_os = "linux")]
impl AgentCgroup {
    fn prepare(root: &Path, session_id: &str) -> Result<Self, AgentRuntimeError> {
        fs::create_dir_all(root).map_err(|error| {
            AgentRuntimeError::Io(format!("creating agent cgroup root failed: {error}"))
        })?;
        let path = root.join(session_id);
        if path.exists() {
            let stale = Self { path: path.clone() };
            stale.kill();
            stale.remove();
        }
        fs::create_dir_all(&path).map_err(|error| {
            AgentRuntimeError::Io(format!("creating agent session cgroup failed: {error}"))
        })?;
        #[cfg(test)]
        create_fake_cgroup_files(root, &path)?;
        if !path.join("cgroup.procs").exists() || !path.join("cgroup.kill").exists() {
            let _ = fs::remove_dir(&path);
            return Err(AgentRuntimeError::Unsupported(
                "agent cgroup root must be a writable cgroup v2 hierarchy with cgroup.kill"
                    .to_string(),
            ));
        }
        Ok(Self { path })
    }

    fn procs_cstring(&self) -> Result<CString, AgentRuntimeError> {
        use std::os::unix::ffi::OsStrExt;

        CString::new(self.path.join("cgroup.procs").as_os_str().as_bytes())
            .map_err(|_| AgentRuntimeError::Io("agent cgroup path contains a NUL byte".to_string()))
    }

    fn kill(&self) {
        if fs::write(self.path.join("cgroup.kill"), b"1\n").is_ok() {
            return;
        }
        for _ in 0..3 {
            let Ok(procs) = fs::read_to_string(self.path.join("cgroup.procs")) else {
                return;
            };
            let mut saw_process = false;
            for line in procs.lines() {
                let Ok(pid) = line.trim().parse::<libc::pid_t>() else {
                    continue;
                };
                saw_process = true;
                unsafe {
                    let _ = libc::kill(pid, libc::SIGKILL);
                }
            }
            if !saw_process {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn remove(&self) {
        if let Err(error) = fs::remove_dir(&self.path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(
                path = %self.path.display(),
                "failed to remove agent cgroup: {error}"
            );
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
fn create_fake_cgroup_files(root: &Path, path: &Path) -> Result<(), AgentRuntimeError> {
    if !root.join(TEST_FAKE_CGROUP_MARKER).exists() {
        return Ok(());
    }
    fs::write(path.join("cgroup.procs"), b"").map_err(|error| {
        AgentRuntimeError::Io(format!("creating fake cgroup.procs failed: {error}"))
    })?;
    fs::write(path.join("cgroup.kill"), b"").map_err(|error| {
        AgentRuntimeError::Io(format!("creating fake cgroup.kill failed: {error}"))
    })?;
    Ok(())
}

#[cfg(target_os = "macos")]
impl AgentCgroup {
    fn prepare(_root: &Path, _session_id: &str) -> Result<Self, AgentRuntimeError> {
        Ok(Self)
    }

    fn kill(&self) {}

    fn remove(&self) {}
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
impl AgentCgroup {
    fn prepare(_root: &Path, _session_id: &str) -> Result<Self, AgentRuntimeError> {
        Err(AgentRuntimeError::Unsupported(
            "chroot agent launch is only supported on Linux and macOS".to_string(),
        ))
    }

    fn kill(&self) {}

    fn remove(&self) {}
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn chown_workspace_for_agent(path: &Path, uid: u32, gid: u32) -> Result<(), AgentRuntimeError> {
    chown_path_for_agent(path, uid, gid)?;
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        AgentRuntimeError::Io(format!("reading agent workspace metadata failed: {error}"))
    })?;
    if !metadata.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(path).map_err(|error| {
        AgentRuntimeError::Io(format!("reading agent workspace failed: {error}"))
    })? {
        let entry = entry.map_err(|error| {
            AgentRuntimeError::Io(format!("reading agent workspace entry failed: {error}"))
        })?;
        chown_workspace_for_agent(&entry.path(), uid, gid)?;
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn chown_path_for_agent(path: &Path, uid: u32, gid: u32) -> Result<(), AgentRuntimeError> {
    use std::os::unix::ffi::OsStrExt;

    let path = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
        AgentRuntimeError::Io("agent workspace path contains a NUL byte".to_string())
    })?;
    if unsafe { libc::lchown(path.as_ptr(), uid, gid) } != 0 {
        return Err(AgentRuntimeError::Io(format!(
            "assigning agent workspace ownership failed: {}",
            std::io::Error::last_os_error()
        )));
    }
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn chown_workspace_for_agent(_path: &Path, _uid: u32, _gid: u32) -> Result<(), AgentRuntimeError> {
    Err(AgentRuntimeError::Unsupported(
        "chroot agent launch is only supported on Linux and macOS".to_string(),
    ))
}

#[cfg(target_os = "linux")]
fn configure_chroot_command(
    command: &mut Command,
    root_dir: &Path,
    cgroup: &AgentCgroup,
    run_uid: u32,
    run_gid: u32,
) -> Result<(), AgentRuntimeError> {
    use std::os::unix::{ffi::OsStrExt, process::CommandExt};

    #[cfg(test)]
    if root_dir.join(TEST_SKIP_CHROOT_PREEXEC_MARKER).exists() {
        return Ok(());
    }

    let root = CString::new(root_dir.as_os_str().as_bytes()).map_err(|_| {
        AgentRuntimeError::Io("agent chroot root path contains a NUL byte".to_string())
    })?;
    let workspace =
        CString::new(CHROOT_CHECKOUT_ROOT).expect("static chroot workspace path has no NUL");
    let cgroup_procs = cgroup.procs_cstring()?;

    unsafe {
        command.pre_exec(move || {
            prepare_chroot_child(&root, &workspace, &cgroup_procs, run_uid, run_gid)
        });
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn prepare_chroot_child(
    root: &CString,
    workspace: &CString,
    cgroup_procs: &CString,
    run_uid: u32,
    run_gid: u32,
) -> std::io::Result<()> {
    move_current_process_to_cgroup(cgroup_procs)?;
    unsafe {
        check_nonnegative_syscall(libc::setsid())?;
        check_zero_syscall(libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0))?;
        check_zero_syscall(libc::chroot(root.as_ptr()))?;
        check_zero_syscall(libc::chdir(workspace.as_ptr()))?;
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

#[cfg(target_os = "macos")]
fn configure_chroot_command(
    command: &mut Command,
    root_dir: &Path,
    _cgroup: &AgentCgroup,
    run_uid: u32,
    run_gid: u32,
) -> Result<(), AgentRuntimeError> {
    use std::os::unix::{ffi::OsStrExt, process::CommandExt};

    let root = CString::new(root_dir.as_os_str().as_bytes()).map_err(|_| {
        AgentRuntimeError::Io("agent chroot root path contains a NUL byte".to_string())
    })?;
    let workspace =
        CString::new(CHROOT_CHECKOUT_ROOT).expect("static chroot workspace path has no NUL");

    unsafe {
        command.pre_exec(move || prepare_macos_chroot_child(&root, &workspace, run_uid, run_gid));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn prepare_macos_chroot_child(
    root: &CString,
    workspace: &CString,
    run_uid: u32,
    run_gid: u32,
) -> std::io::Result<()> {
    unsafe {
        check_nonnegative_syscall(libc::setsid())?;
        check_zero_syscall(libc::chroot(root.as_ptr()))?;
        check_zero_syscall(libc::chdir(workspace.as_ptr()))?;
        check_zero_syscall(libc::setgroups(0, std::ptr::null()))?;
        check_zero_syscall(libc::setgid(run_gid))?;
        check_zero_syscall(libc::setuid(run_uid))?;
    }
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn configure_chroot_command(
    _command: &mut Command,
    _root_dir: &Path,
    _cgroup: &AgentCgroup,
    _run_uid: u32,
    _run_gid: u32,
) -> Result<(), AgentRuntimeError> {
    Err(AgentRuntimeError::Unsupported(
        "chroot agent launch is only supported on Linux and macOS".to_string(),
    ))
}

#[cfg(target_os = "linux")]
fn move_current_process_to_cgroup(cgroup_procs: &CString) -> std::io::Result<()> {
    unsafe {
        let fd = libc::open(cgroup_procs.as_ptr(), libc::O_WRONLY | libc::O_CLOEXEC);
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let pid = libc::getpid();
        let mut buffer = [0_u8; 32];
        let last = buffer.len() - 1;
        let mut cursor = last;
        buffer[last] = b'\n';
        let mut value = pid as u32;
        if value == 0 {
            cursor -= 1;
            buffer[cursor] = b'0';
        } else {
            while value > 0 {
                cursor -= 1;
                buffer[cursor] = b'0' + (value % 10) as u8;
                value /= 10;
            }
        }
        let bytes = &buffer[cursor..];
        let written = libc::write(fd, bytes.as_ptr().cast(), bytes.len());
        let close_result = libc::close(fd);
        if written != bytes.len() as isize {
            return Err(std::io::Error::last_os_error());
        }
        if close_result != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }
}

fn spawn_with_timeout(
    mut command: Command,
    timeout: Duration,
    cgroup: Option<AgentCgroup>,
    port_reservation: Option<TcpListener>,
) -> Result<AgentChild, AgentRuntimeError> {
    let (sender, receiver) = std::sync::mpsc::sync_channel(0);
    let cgroup_for_timeout = cgroup.clone();
    std::thread::spawn(move || {
        let spawn_result = spawn_agent_child(&mut command, cgroup.clone(), port_reservation);
        if let Err(std::sync::mpsc::SendError(result)) = sender.send(spawn_result) {
            cleanup_unsent_spawn_result(result, cgroup.as_ref());
        }
    });
    match receiver.recv_timeout(timeout) {
        Ok(result) => result.map_err(|error| {
            AgentRuntimeError::Io(format!("spawning agent process failed: {error}"))
        }),
        Err(_) => launch_timed_out(timeout, cgroup_for_timeout.as_ref()),
    }
}

fn spawn_agent_child(
    command: &mut Command,
    cgroup: Option<AgentCgroup>,
    port_reservation: Option<TcpListener>,
) -> std::io::Result<AgentChild> {
    let spawn_result = command.spawn();
    drop(port_reservation);
    match spawn_result {
        Ok(child) => Ok(AgentChild {
            child,
            cgroup,
            metadata: AgentLaunchMetadata::default(),
        }),
        Err(error) => {
            if let Some(cgroup) = cgroup.as_ref() {
                cgroup.remove();
            }
            Err(error)
        }
    }
}

fn cleanup_unsent_spawn_result(result: std::io::Result<AgentChild>, cgroup: Option<&AgentCgroup>) {
    match result {
        Ok(mut child) => terminate_agent_child(&mut child),
        Err(_) => {
            if let Some(cgroup) = cgroup {
                cgroup.remove();
            }
        }
    }
}

fn launch_timed_out(
    timeout: Duration,
    cgroup: Option<&AgentCgroup>,
) -> Result<AgentChild, AgentRuntimeError> {
    if let Some(cgroup) = cgroup {
        cgroup.kill();
    }
    Err(AgentRuntimeError::LaunchTimedOut(timeout))
}

fn terminate_agent_child(child: &mut AgentChild) {
    if let Some(cgroup) = child.cgroup.as_ref() {
        cgroup.kill();
    }
    terminate_child(&mut child.child);
    if let Some(cgroup) = child.cgroup.as_ref() {
        cgroup.remove();
    }
}

#[cfg(unix)]
fn terminate_child(child: &mut Child) {
    let child_pid = child.id() as libc::pid_t;
    unsafe {
        let _ = libc::kill(-child_pid, libc::SIGKILL);
    }
    match child.try_wait() {
        Ok(Some(_)) => {}
        Ok(None) | Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[cfg(not(unix))]
fn terminate_child(child: &mut Child) {
    match child.try_wait() {
        Ok(Some(_)) => {}
        Ok(None) | Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_checkout() -> PreparedWorkspaceCheckout {
        PreparedWorkspaceCheckout {
            checkout_relpath: "agent-runtimes/s_test/root/workspace".to_string(),
            checkout_ref: None,
            checkout_commit_sha: None,
            working_dir: PathBuf::from("/state/agent-runtimes/s_test/root/workspace"),
        }
    }

    fn temp_state_dir(prefix: &str) -> PathBuf {
        std::env::temp_dir().join(format!("{prefix}-{}", uuid::Uuid::new_v4().simple()))
    }

    fn checkout_in_state(state_dir: &Path, session_id: &str) -> PreparedWorkspaceCheckout {
        let root_dir = FsAgentRuntimeManager::root_dir_for_state(state_dir, session_id);
        let working_dir = root_dir.join("workspace");
        std::fs::create_dir_all(&working_dir).expect("checkout fixture should be creatable");
        PreparedWorkspaceCheckout {
            checkout_relpath: format!("agent-runtimes/{session_id}/root/workspace"),
            checkout_ref: None,
            checkout_commit_sha: None,
            working_dir,
        }
    }

    #[cfg(unix)]
    fn test_agent_identity() -> (u32, u32) {
        unsafe {
            let uid = libc::geteuid() as u32;
            let gid = libc::getegid() as u32;
            if uid == 0 {
                (DEFAULT_AGENT_RUN_UID, DEFAULT_AGENT_RUN_GID)
            } else {
                (uid, gid)
            }
        }
    }

    #[cfg(target_os = "linux")]
    fn fake_cgroup_root() -> PathBuf {
        let root = temp_state_dir("acp-agent-fake-cgroup");
        std::fs::create_dir_all(&root).expect("fake cgroup root should be creatable");
        std::fs::write(root.join(TEST_FAKE_CGROUP_MARKER), b"")
            .expect("fake cgroup marker should be writable");
        root
    }

    #[cfg(target_os = "linux")]
    fn python3_path() -> String {
        for candidate in [
            "/usr/bin/python3",
            "/usr/local/bin/python3",
            "/opt/yamllint/bin/python3",
        ] {
            if Path::new(candidate).exists() {
                return candidate.to_string();
            }
        }
        std::env::var_os("PATH")
            .and_then(|path| {
                std::env::split_paths(&path)
                    .map(|dir| dir.join("python3"))
                    .find(|candidate| candidate.exists())
            })
            .expect("python3 should be available")
            .display()
            .to_string()
    }

    #[test]
    fn launch_config_validation_requires_command() {
        assert_eq!(
            AgentLaunchConfig::chroot(
                Vec::new(),
                Vec::new(),
                DEFAULT_AGENT_LAUNCH_TIMEOUT,
                DEFAULT_AGENT_RUN_UID,
                DEFAULT_AGENT_RUN_GID,
            )
            .expect_err("missing commands should fail"),
            AgentLaunchConfigError::MissingCommand
        );
    }

    #[test]
    fn launch_config_validation_rejects_empty_argv_element() {
        assert_eq!(
            AgentLaunchConfig::chroot(
                vec!["agent".to_string(), String::new()],
                Vec::new(),
                DEFAULT_AGENT_LAUNCH_TIMEOUT,
                DEFAULT_AGENT_RUN_UID,
                DEFAULT_AGENT_RUN_GID,
            )
            .expect_err("empty argv should fail"),
            AgentLaunchConfigError::EmptyArgvElement
        );
    }

    #[test]
    fn launch_config_validation_rejects_unsafe_env_name() {
        assert_eq!(
            AgentLaunchConfig::chroot(
                vec!["agent".to_string()],
                vec!["bad-name".to_string()],
                DEFAULT_AGENT_LAUNCH_TIMEOUT,
                DEFAULT_AGENT_RUN_UID,
                DEFAULT_AGENT_RUN_GID,
            )
            .expect_err("unsafe env names should fail"),
            AgentLaunchConfigError::InvalidEnvName("bad-name".to_string())
        );
    }

    #[test]
    fn launch_config_validation_accepts_safe_env_names() {
        let config = AgentLaunchConfig::chroot(
            vec!["agent".to_string()],
            vec!["_ACP_OK".to_string(), "ACP1".to_string()],
            DEFAULT_AGENT_LAUNCH_TIMEOUT,
            DEFAULT_AGENT_RUN_UID,
            DEFAULT_AGENT_RUN_GID,
        )
        .expect("safe env names should validate");

        assert_eq!(config.mode.as_str(), "chroot");
    }

    #[test]
    fn launch_config_validation_rejects_empty_env_name() {
        assert_eq!(
            AgentLaunchConfig::chroot(
                vec!["agent".to_string()],
                vec![String::new()],
                DEFAULT_AGENT_LAUNCH_TIMEOUT,
                DEFAULT_AGENT_RUN_UID,
                DEFAULT_AGENT_RUN_GID,
            )
            .expect_err("empty env names should fail"),
            AgentLaunchConfigError::InvalidEnvName(String::new())
        );
    }

    #[test]
    fn launch_config_validation_requires_nonzero_timeout() {
        assert_eq!(
            AgentLaunchConfig::chroot(
                vec!["agent".to_string()],
                Vec::new(),
                Duration::ZERO,
                DEFAULT_AGENT_RUN_UID,
                DEFAULT_AGENT_RUN_GID,
            )
            .expect_err("zero timeout should fail"),
            AgentLaunchConfigError::InvalidTimeout
        );
    }

    #[test]
    fn launch_config_validation_rejects_root_uid() {
        assert_eq!(
            AgentLaunchConfig::chroot(
                vec!["agent".to_string()],
                Vec::new(),
                DEFAULT_AGENT_LAUNCH_TIMEOUT,
                0,
                DEFAULT_AGENT_RUN_GID,
            )
            .expect_err("root run uid should fail"),
            AgentLaunchConfigError::RootRunUid
        );
    }

    #[test]
    fn launch_config_validation_rejects_root_gid() {
        assert_eq!(
            AgentLaunchConfig::chroot(
                vec!["agent".to_string()],
                Vec::new(),
                DEFAULT_AGENT_LAUNCH_TIMEOUT,
                DEFAULT_AGENT_RUN_UID,
                0,
            )
            .expect_err("root run gid should fail"),
            AgentLaunchConfigError::RootRunGid
        );
    }

    #[test]
    fn noop_runtime_manager_launches_and_forgets_without_side_effects() {
        let manager = NoopAgentRuntimeManager;
        manager
            .launch_session(&AgentSessionLaunch {
                session_id: "s_test",
                workspace_id: "w_test",
                checkout: &sample_checkout(),
                config: None,
            })
            .expect("noop launch should succeed");
        manager.forget_session("s_test");
    }

    #[test]
    fn runtime_error_display_messages_are_specific() {
        assert_eq!(
            AgentLaunchConfigError::EmptyArgvElement.to_string(),
            "agent command argv elements must not be empty"
        );
        assert_eq!(
            AgentLaunchConfigError::InvalidTimeout.to_string(),
            "agent launch timeout must be greater than zero"
        );
        assert_eq!(
            AgentLaunchConfigError::RootRunUid.to_string(),
            "agent run uid must be non-root"
        );
        assert_eq!(
            AgentLaunchConfigError::RootRunGid.to_string(),
            "agent run gid must be non-root"
        );
        assert_eq!(
            AgentLaunchConfigError::InvalidEnvName("bad-name".to_string()).to_string(),
            "agent env allowlist name is invalid: bad-name"
        );
        assert_eq!(
            AgentRuntimeError::from(AgentLaunchConfigError::MissingCommand).to_string(),
            "agent command is required in chroot mode"
        );
        assert_eq!(
            AgentRuntimeError::Io("io failed".to_string()).to_string(),
            "io failed"
        );
        assert_eq!(
            AgentRuntimeError::Unsupported("unsupported".to_string()).to_string(),
            "unsupported"
        );
        assert_eq!(
            AgentRuntimeError::AlreadyRunning("s_test".to_string()).to_string(),
            "agent runtime already exists for session s_test"
        );
        assert_eq!(
            AgentRuntimeError::LaunchTimedOut(Duration::from_secs(2)).to_string(),
            "agent launch timed out after 2s"
        );
    }

    #[derive(Debug)]
    struct PanicAgentRuntimeManager;

    impl AgentRuntimeManager for PanicAgentRuntimeManager {
        fn launch_session(
            &self,
            _launch: &AgentSessionLaunch<'_>,
        ) -> Result<AgentLaunchMetadata, AgentRuntimeError> {
            panic!("join failure fixture");
        }

        fn forget_session(&self, _session_id: &str) {}
    }

    #[tokio::test]
    async fn launch_session_blocking_reports_join_failures() {
        let error = launch_session_blocking(
            Arc::new(PanicAgentRuntimeManager),
            "s_test".to_string(),
            "w_test".to_string(),
            sample_checkout(),
            None,
        )
        .await
        .expect_err("panic should become join failure");

        assert!(matches!(
            error,
            AgentRuntimeError::Io(message)
                if message.starts_with("joining agent runtime launch failed:")
        ));
    }

    #[derive(Debug)]
    struct MetadataAgentRuntimeManager;

    impl AgentRuntimeManager for MetadataAgentRuntimeManager {
        fn launch_session(
            &self,
            _launch: &AgentSessionLaunch<'_>,
        ) -> Result<AgentLaunchMetadata, AgentRuntimeError> {
            Ok(AgentLaunchMetadata {
                acp_address: Some("127.0.0.1:4567".to_string()),
            })
        }

        fn forget_session(&self, _session_id: &str) {}
    }

    #[tokio::test]
    async fn launch_session_blocking_returns_runtime_metadata() {
        let metadata = launch_session_blocking(
            Arc::new(MetadataAgentRuntimeManager),
            "s_test".to_string(),
            "w_test".to_string(),
            sample_checkout(),
            None,
        )
        .await
        .expect("metadata launch should succeed");

        assert_eq!(metadata.acp_address.as_deref(), Some("127.0.0.1:4567"));
    }

    #[test]
    fn argv_placeholders_expand_only_when_endpoint_allocated() {
        let endpoint = AcpEndpoint {
            address: "127.0.0.1:4567".to_string(),
            port: 4567,
        };
        let argv = expand_agent_argv(
            &[
                "agent".to_string(),
                "--port=${ACP_PORT}".to_string(),
                "--url=${ACP_BASE_URL}".to_string(),
                "--host=${ACP_HOST}".to_string(),
                "--endpoint=${ACP_ENDPOINT}".to_string(),
            ],
            Some(&endpoint),
        );

        assert_eq!(
            argv,
            vec![
                "agent",
                "--port=4567",
                "--url=http://127.0.0.1:4567",
                "--host=127.0.0.1",
                "--endpoint=127.0.0.1:4567"
            ]
        );
    }

    #[test]
    fn argv_placeholders_remain_literal_without_endpoint() {
        let command = vec!["agent".to_string(), "--port=${ACP_PORT}".to_string()];

        assert_eq!(expand_agent_argv(&command, None), command);
    }

    #[test]
    fn detects_acp_placeholders_in_argv() {
        assert!(contains_acp_placeholder("--port=${ACP_PORT}"));
        assert!(contains_acp_placeholder("${ACP_ENDPOINT}"));
        assert!(contains_acp_placeholder("${ACP_BASE_URL}"));
        assert!(contains_acp_placeholder("${ACP_HOST}"));
        assert!(!contains_acp_placeholder("--stdio"));
    }

    #[test]
    fn acp_endpoint_is_reserved_only_when_command_uses_placeholders() {
        assert!(
            AcpEndpoint::reserve_for_command(&["agent".to_string()])
                .expect("reservation check should succeed")
                .is_none()
        );
        let reserved = AcpEndpoint::reserve_for_command(&["${ACP_PORT}".to_string()])
            .expect("placeholder command should reserve a port")
            .expect("reservation should be present");

        assert_eq!(
            reserved.endpoint.address,
            format!("127.0.0.1:{}", reserved.endpoint.port)
        );
        drop(reserved);
    }

    #[test]
    fn build_agent_command_injects_structured_env_and_allowed_values() {
        let (env_name, command) = built_agent_command_fixture();

        assert_eq!(command.get_program(), "/bin/echo");
        assert_eq!(command.get_args().collect::<Vec<_>>(), vec!["4567"]);
        let envs = command_envs(&command);
        assert_eq!(
            envs.get(&env_name).and_then(|value| value.as_deref()),
            Some(std::ffi::OsStr::new("allowed"))
        );
        assert_eq!(
            envs.get("ACP_SESSION_ID")
                .and_then(|value| value.as_deref()),
            Some(std::ffi::OsStr::new("s_test"))
        );
        assert_eq!(
            envs.get("ACP_PORT").and_then(|value| value.as_deref()),
            Some(std::ffi::OsStr::new("4567"))
        );
    }

    fn built_agent_command_fixture() -> (String, Command) {
        let env_name = format!("ACP_TEST_ALLOWED_{}", uuid::Uuid::new_v4().simple());
        unsafe {
            std::env::set_var(&env_name, "allowed");
        }
        let config = AgentLaunchConfig::chroot(
            vec!["/bin/echo".to_string(), "${ACP_PORT}".to_string()],
            vec![env_name.clone()],
            DEFAULT_AGENT_LAUNCH_TIMEOUT,
            DEFAULT_AGENT_RUN_UID,
            DEFAULT_AGENT_RUN_GID,
        )
        .expect("config should validate");
        let checkout = sample_checkout();
        let launch = AgentSessionLaunch {
            session_id: "s_test",
            workspace_id: "w_test",
            checkout: &checkout,
            config: Some(config.clone()),
        };
        let command = build_agent_command(&config, &launch, Some(&endpoint_fixture()));
        unsafe {
            std::env::remove_var(&env_name);
        }
        (env_name, command)
    }

    fn endpoint_fixture() -> AcpEndpoint {
        AcpEndpoint {
            address: "127.0.0.1:4567".to_string(),
            port: 4567,
        }
    }

    fn command_envs(command: &Command) -> HashMap<String, Option<std::ffi::OsString>> {
        command
            .get_envs()
            .map(|(name, value)| {
                (
                    name.to_string_lossy().into_owned(),
                    value.map(|value| value.to_os_string()),
                )
            })
            .collect()
    }

    #[test]
    fn runtime_forget_removes_runtime_directory_without_launching() {
        let state_dir = temp_state_dir("acp-agent-runtime-cleanup");
        let runtime_dir = FsAgentRuntimeManager::runtime_dir_for_state(&state_dir, "s_test");
        std::fs::create_dir_all(runtime_dir.join("root/workspace"))
            .expect("runtime fixture should be creatable");
        let manager = FsAgentRuntimeManager::new(state_dir, None)
            .expect("manager without launch config should build");

        manager.forget_session("s_test");

        assert!(!runtime_dir.exists(), "forget should remove runtime roots");
    }

    #[test]
    fn runtime_paths_are_scoped_by_state_and_session() {
        let state_dir = PathBuf::from("/state");
        let manager =
            FsAgentRuntimeManager::new(state_dir.clone(), None).expect("manager should build");

        assert_eq!(
            FsAgentRuntimeManager::runtime_dir_for_state(&state_dir, "s_test"),
            PathBuf::from("/state/agent-runtimes/s_test")
        );
        assert_eq!(
            FsAgentRuntimeManager::root_dir_for_state(&state_dir, "s_test"),
            PathBuf::from("/state/agent-runtimes/s_test/root")
        );
        assert_eq!(
            manager.runtime_dir("s_test"),
            PathBuf::from("/state/agent-runtimes/s_test")
        );
        assert_eq!(
            manager.root_dir("s_test"),
            PathBuf::from("/state/agent-runtimes/s_test/root")
        );
    }

    #[test]
    fn manager_new_validates_config() {
        let error = FsAgentRuntimeManager::new_with_cgroup_root(
            temp_state_dir("acp-agent-invalid-manager-config"),
            Some(AgentLaunchConfig {
                mode: AgentLaunchMode::Chroot,
                command: Vec::new(),
                env_allowlist: Vec::new(),
                timeout: DEFAULT_AGENT_LAUNCH_TIMEOUT,
                run_uid: DEFAULT_AGENT_RUN_UID,
                run_gid: DEFAULT_AGENT_RUN_GID,
            }),
            None,
        )
        .expect_err("invalid manager config should fail");

        assert!(matches!(
            error,
            AgentRuntimeError::Config(AgentLaunchConfigError::MissingCommand)
        ));
    }

    #[test]
    fn fs_runtime_without_config_returns_default_metadata() {
        let manager = FsAgentRuntimeManager::new(temp_state_dir("acp-agent-no-config"), None)
            .expect("manager should build");
        let checkout = sample_checkout();
        let metadata = manager
            .launch_session(&AgentSessionLaunch {
                session_id: "s_test",
                workspace_id: "w_test",
                checkout: &checkout,
                config: None,
            })
            .expect("missing config should be a noop");

        assert_eq!(metadata, AgentLaunchMetadata::default());
    }

    #[cfg(unix)]
    fn process_is_running(pid: u32) -> bool {
        unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
    }

    #[cfg(unix)]
    #[test]
    fn runtime_forget_kills_tracked_children() {
        let manager = FsAgentRuntimeManager::new(
            std::env::temp_dir().join(format!(
                "acp-agent-runtime-child-cleanup-{}",
                uuid::Uuid::new_v4().simple()
            )),
            None,
        )
        .expect("manager should build");
        let child = Command::new("/bin/sleep")
            .arg("60")
            .spawn()
            .expect("sleep child should spawn");
        let pid = child.id();
        manager
            .children
            .lock()
            .expect("child registry should not poison")
            .slots
            .insert(
                "s_test".to_string(),
                AgentChildSlot::Running(AgentChild {
                    child,
                    cgroup: None,
                    metadata: AgentLaunchMetadata::default(),
                }),
            );

        manager.forget_session("s_test");

        assert!(!process_is_running(pid), "forget should terminate children");
    }

    #[cfg(unix)]
    #[test]
    fn running_launch_returns_existing_metadata() {
        let manager = FsAgentRuntimeManager::new(
            std::env::temp_dir().join(format!(
                "acp-agent-runtime-existing-{}",
                uuid::Uuid::new_v4().simple()
            )),
            None,
        )
        .expect("manager should build");
        let child = Command::new("/bin/sleep")
            .arg("60")
            .spawn()
            .expect("sleep child should spawn");
        let metadata = AgentLaunchMetadata {
            acp_address: Some("127.0.0.1:49152".to_string()),
        };
        manager
            .children
            .lock()
            .expect("child registry should not poison")
            .slots
            .insert(
                "s_test".to_string(),
                AgentChildSlot::Running(AgentChild {
                    child,
                    cgroup: None,
                    metadata: metadata.clone(),
                }),
            );
        let config = AgentLaunchConfig::chroot(
            vec!["/bin/true".to_string()],
            Vec::new(),
            DEFAULT_AGENT_LAUNCH_TIMEOUT,
            DEFAULT_AGENT_RUN_UID,
            DEFAULT_AGENT_RUN_GID,
        )
        .expect("config should validate");

        let restored = manager
            .launch_session(&AgentSessionLaunch {
                session_id: "s_test",
                workspace_id: "w_test",
                checkout: &sample_checkout(),
                config: Some(config),
            })
            .expect("existing launch should return metadata");
        manager.forget_session("s_test");

        assert_eq!(restored, metadata);
    }

    #[cfg(unix)]
    #[test]
    fn runtime_drop_kills_tracked_children() {
        let manager = FsAgentRuntimeManager::new(
            std::env::temp_dir().join(format!(
                "acp-agent-runtime-drop-cleanup-{}",
                uuid::Uuid::new_v4().simple()
            )),
            None,
        )
        .expect("manager should build");
        let child = Command::new("/bin/sleep")
            .arg("60")
            .spawn()
            .expect("sleep child should spawn");
        let pid = child.id();
        manager
            .children
            .lock()
            .expect("child registry should not poison")
            .slots
            .insert(
                "s_test".to_string(),
                AgentChildSlot::Running(AgentChild {
                    child,
                    cgroup: None,
                    metadata: AgentLaunchMetadata::default(),
                }),
            );

        drop(manager);

        assert!(!process_is_running(pid), "drop should terminate children");
    }

    #[test]
    fn concurrent_launch_waits_for_in_flight_reservation() {
        let manager = Arc::new(
            FsAgentRuntimeManager::new(PathBuf::from("/tmp/acp-agent-runtime-wait"), None)
                .expect("manager should build"),
        );
        let launch_id = manager
            .reserve_launch("s_test")
            .expect("first launch should reserve");
        let waiting_manager = manager.clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        let waiter = std::thread::spawn(move || {
            let result = waiting_manager.reserve_launch("s_test");
            sender
                .send(result)
                .expect("wait result should send back to test");
        });

        assert!(
            receiver.recv_timeout(Duration::from_millis(50)).is_err(),
            "second launch should wait while the first launch is in flight"
        );

        manager.clear_launch_reservation("s_test", launch_id);
        let second_launch_id = receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("second launch should proceed after first reservation clears")
            .expect("second launch should reserve");
        manager.clear_launch_reservation("s_test", second_launch_id);
        waiter.join().expect("waiter should finish");
    }

    #[test]
    fn running_launch_metadata_errors_when_session_is_not_running() {
        let manager = FsAgentRuntimeManager::new(temp_state_dir("acp-agent-no-running"), None)
            .expect("manager should build");
        let error = manager
            .running_launch_metadata("s_test")
            .expect_err("missing running child should fail");

        assert!(matches!(error, AgentRuntimeError::AlreadyRunning(session) if session == "s_test"));
    }

    #[cfg(unix)]
    #[test]
    fn store_launched_child_cleans_up_when_reservation_disappears() {
        let manager = FsAgentRuntimeManager::new(temp_state_dir("acp-agent-cancelled-store"), None)
            .expect("manager should build");
        let child = Command::new("/bin/sleep")
            .arg("60")
            .spawn()
            .expect("sleep child should spawn");
        let pid = child.id();
        let error = manager
            .store_launched_child(
                "s_test",
                7,
                AgentChild {
                    child,
                    cgroup: None,
                    metadata: AgentLaunchMetadata::default(),
                },
            )
            .expect_err("missing reservation should fail");

        assert!(
            matches!(error, AgentRuntimeError::Io(message) if message == "agent runtime launch was cancelled")
        );
        assert!(
            !process_is_running(pid),
            "cancelled store should kill child"
        );
    }

    #[cfg(unix)]
    #[test]
    fn store_launched_child_handles_mismatched_and_running_slots() {
        let manager =
            FsAgentRuntimeManager::new(temp_state_dir("acp-agent-mismatched-store"), None)
                .expect("manager should build");
        manager
            .children
            .lock()
            .expect("registry should lock")
            .slots
            .insert("s_test".to_string(), AgentChildSlot::Launching(1));
        let mismatched = manager.store_launched_child("s_test", 2, spawn_sleep_child());
        assert!(
            matches!(mismatched, Err(AgentRuntimeError::Io(message)) if message == "agent runtime launch was cancelled")
        );

        let running_child = spawn_sleep_child();
        manager
            .children
            .lock()
            .expect("registry should lock")
            .slots
            .insert("s_test".to_string(), AgentChildSlot::Running(running_child));
        let running = manager.store_launched_child("s_test", 3, spawn_sleep_child());
        assert!(
            matches!(running, Err(AgentRuntimeError::AlreadyRunning(session)) if session == "s_test")
        );
        manager.forget_session("s_test");
    }

    #[cfg(unix)]
    #[test]
    fn wait_for_acp_readiness_handles_ready_exit_and_timeout_paths() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("listener should bind");
        let endpoint = AcpEndpoint {
            address: listener.local_addr().expect("listener address").to_string(),
            port: listener.local_addr().expect("listener address").port(),
        };
        let mut child = spawn_sleep_child();
        wait_for_acp_readiness(Some(&endpoint), Duration::from_secs(1), &mut child)
            .expect("listening endpoint should be ready");
        terminate_agent_child(&mut child);

        let mut exited = spawn_true_child();
        let error = wait_for_acp_readiness(
            Some(&closed_endpoint()),
            Duration::from_secs(1),
            &mut exited,
        )
        .expect_err("exited child should fail readiness");
        assert!(
            matches!(error, AgentRuntimeError::Io(message) if message.contains("exited before ACP endpoint"))
        );

        let mut sleeping = spawn_sleep_child();
        let error = wait_for_acp_readiness(
            Some(&closed_endpoint()),
            Duration::from_millis(1),
            &mut sleeping,
        )
        .expect_err("sleeping child should time out");
        terminate_agent_child(&mut sleeping);
        assert!(matches!(error, AgentRuntimeError::LaunchTimedOut(_)));
    }

    #[cfg(unix)]
    #[test]
    fn wait_for_acp_readiness_accepts_missing_endpoint_and_bad_addresses_fail() {
        let mut child = spawn_sleep_child();
        wait_for_acp_readiness(None, Duration::from_millis(1), &mut child)
            .expect("missing endpoint should not wait");
        let error = wait_for_acp_readiness(
            Some(&AcpEndpoint {
                address: "not-an-address".to_string(),
                port: 0,
            }),
            Duration::from_millis(1),
            &mut child,
        )
        .expect_err("bad endpoint address should fail");
        terminate_agent_child(&mut child);

        assert!(
            matches!(error, AgentRuntimeError::Io(message) if message.contains("parsing ACP endpoint"))
        );
    }

    #[cfg(unix)]
    fn spawn_sleep_child() -> AgentChild {
        AgentChild {
            child: Command::new("/bin/sleep")
                .arg("60")
                .spawn()
                .expect("sleep child should spawn"),
            cgroup: None,
            metadata: AgentLaunchMetadata::default(),
        }
    }

    #[cfg(unix)]
    fn spawn_true_child() -> AgentChild {
        AgentChild {
            child: Command::new("/bin/true")
                .spawn()
                .expect("true child should spawn"),
            cgroup: None,
            metadata: AgentLaunchMetadata::default(),
        }
    }

    fn closed_endpoint() -> AcpEndpoint {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("listener should bind");
        let address = listener.local_addr().expect("listener address");
        drop(listener);
        AcpEndpoint {
            address: address.to_string(),
            port: address.port(),
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn prepare_cgroup_supports_fake_cgroup_fixture() {
        let root = fake_cgroup_root();
        let cgroup = AgentCgroup::prepare(&root, "s_test").expect("fake cgroup should prepare");

        assert!(cgroup.path.join("cgroup.procs").exists());
        assert!(cgroup.path.join("cgroup.kill").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn prepare_cgroup_cleans_stale_fake_cgroups() {
        let root = fake_cgroup_root();
        let stale = root.join("s_test");
        std::fs::create_dir_all(&stale).expect("stale cgroup should be creatable");
        std::fs::write(stale.join("cgroup.procs"), b"").expect("stale procs should write");
        std::fs::write(stale.join("cgroup.kill"), b"").expect("stale kill should write");

        let cgroup = AgentCgroup::prepare(&root, "s_test").expect("fake cgroup should prepare");

        assert!(cgroup.path.exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn prepare_cgroup_rejects_plain_directories() {
        let root = temp_state_dir("acp-agent-plain-cgroup");
        let error =
            AgentCgroup::prepare(&root, "s_test").expect_err("plain directories are not cgroup v2");

        assert!(
            matches!(error, AgentRuntimeError::Unsupported(message) if message.contains("cgroup v2"))
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn prepare_session_cgroup_requires_linux_root() {
        let manager = FsAgentRuntimeManager::new_with_cgroup_root(
            temp_state_dir("acp-agent-no-cgroup-root"),
            None,
            None,
        )
        .expect("manager should build");
        let error = manager
            .prepare_session_cgroup("s_test")
            .expect_err("missing cgroup root should fail");

        assert!(
            matches!(error, AgentRuntimeError::Unsupported(message) if message.contains("requires a Linux cgroup"))
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn configure_chroot_command_accepts_fake_cgroup_without_spawning() {
        let root_dir = temp_state_dir("acp-agent-configure-chroot");
        std::fs::create_dir_all(&root_dir).expect("root fixture should be creatable");
        let cgroup_root = fake_cgroup_root();
        let cgroup = AgentCgroup::prepare(&cgroup_root, "s_test").expect("cgroup should prepare");
        let (uid, gid) = test_agent_identity();
        let mut command = Command::new("/bin/true");

        configure_chroot_command(&mut command, &root_dir, &cgroup, uid, gid)
            .expect("valid chroot command should configure");

        let _ = std::fs::remove_dir_all(root_dir);
        let _ = std::fs::remove_dir_all(cgroup_root);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn syscall_helpers_cover_success_and_error_results() {
        assert!(check_nonnegative_syscall(0).is_ok());
        assert!(check_nonnegative_syscall(-1).is_err());
        assert!(check_zero_syscall(0).is_ok());
        assert!(check_zero_syscall(-1).is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn move_current_process_to_cgroup_writes_pid_to_path() {
        let path = temp_state_dir("acp-agent-cgroup-procs").join("cgroup.procs");
        std::fs::create_dir_all(path.parent().expect("procs parent"))
            .expect("procs parent should be creatable");
        std::fs::write(&path, b"").expect("procs file should be writable");
        let cgroup_procs = CString::new(path.as_os_str().as_encoded_bytes())
            .expect("test path should not contain NUL");

        move_current_process_to_cgroup(&cgroup_procs).expect("pid should write");

        assert!(
            !std::fs::read_to_string(&path)
                .expect("procs should read")
                .is_empty()
        );
        let _ = std::fs::remove_dir_all(path.parent().expect("procs parent"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn move_current_process_to_cgroup_reports_open_failures() {
        let path = temp_state_dir("acp-agent-missing-cgroup-procs").join("missing");
        let cgroup_procs = CString::new(path.as_os_str().as_encoded_bytes())
            .expect("test path should not contain NUL");

        assert!(move_current_process_to_cgroup(&cgroup_procs).is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn cgroup_kill_falls_back_to_listed_processes() {
        let cgroup_path = temp_state_dir("acp-agent-cgroup-kill-fallback");
        std::fs::create_dir_all(&cgroup_path).expect("fake cgroup should be creatable");
        std::fs::create_dir(cgroup_path.join("cgroup.kill"))
            .expect("fake cgroup.kill directory should be creatable");
        let mut child = Command::new("/bin/sleep")
            .arg("60")
            .spawn()
            .expect("sleep child should spawn");
        let pid = child.id();
        std::fs::write(
            cgroup_path.join("cgroup.procs"),
            format!("invalid\n{pid}\n"),
        )
        .expect("fake cgroup.procs should be writable");
        let cgroup = AgentCgroup { path: cgroup_path };

        cgroup.kill();
        let _ = child.wait();

        assert!(
            !process_is_running(pid),
            "fallback kill should terminate child"
        );
        let _ = std::fs::remove_dir_all(cgroup.path);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn chown_workspace_handles_files_and_nested_directories_for_current_identity() {
        let (uid, gid) = test_agent_identity();
        let workspace = temp_state_dir("acp-agent-chown-current");
        let nested = workspace.join("nested");
        std::fs::create_dir_all(&nested).expect("workspace fixture should be creatable");
        std::fs::write(nested.join("file.txt"), b"data")
            .expect("workspace file should be writable");

        chown_workspace_for_agent(&workspace, uid, gid).expect("current owner chown should work");
        chown_workspace_for_agent(&nested.join("file.txt"), uid, gid)
            .expect("file chown should work");

        let _ = std::fs::remove_dir_all(workspace);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn chown_workspace_reports_missing_paths() {
        let (uid, gid) = test_agent_identity();
        let error = chown_workspace_for_agent(&temp_state_dir("acp-agent-missing-chown"), uid, gid)
            .expect_err("missing chown path should fail");

        assert!(
            matches!(error, AgentRuntimeError::Io(message) if message.contains("assigning agent workspace ownership failed"))
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn launch_chroot_binds_reserved_endpoint_with_fake_cgroup() {
        let state_dir = temp_state_dir("acp-agent-launch-chroot");
        let session_id = "s_test";
        let checkout = checkout_in_state(&state_dir, session_id);
        let root_dir = FsAgentRuntimeManager::root_dir_for_state(&state_dir, session_id);
        std::fs::write(root_dir.join(TEST_SKIP_CHROOT_PREEXEC_MARKER), b"")
            .expect("skip marker should be writable");
        let cgroup_root = fake_cgroup_root();
        let manager = FsAgentRuntimeManager::new_with_cgroup_root(
            state_dir.clone(),
            None,
            Some(cgroup_root.clone()),
        )
        .expect("manager should build");
        let config = python_acp_server_config();

        let metadata = manager
            .launch_session(&AgentSessionLaunch {
                session_id,
                workspace_id: "w_test",
                checkout: &checkout,
                config: Some(config),
            })
            .expect("fake chroot launch should succeed");

        assert!(metadata.acp_address.as_deref().is_some_and(|address| {
            address.starts_with("127.0.0.1:") && address.len() > "127.0.0.1:".len()
        }));
        manager.forget_session(session_id);
        let _ = std::fs::remove_dir_all(state_dir);
        let _ = std::fs::remove_dir_all(cgroup_root);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn launch_chroot_cleans_up_when_endpoint_never_becomes_ready() {
        let state_dir = temp_state_dir("acp-agent-launch-not-ready");
        let session_id = "s_test";
        let checkout = checkout_in_state(&state_dir, session_id);
        let root_dir = FsAgentRuntimeManager::root_dir_for_state(&state_dir, session_id);
        std::fs::write(root_dir.join(TEST_SKIP_CHROOT_PREEXEC_MARKER), b"")
            .expect("skip marker should be writable");
        let cgroup_root = fake_cgroup_root();
        let manager = FsAgentRuntimeManager::new_with_cgroup_root(
            state_dir.clone(),
            None,
            Some(cgroup_root.clone()),
        )
        .expect("manager should build");
        let (uid, gid) = test_agent_identity();
        let config = AgentLaunchConfig::chroot(
            vec!["/bin/true".to_string(), "${ACP_PORT}".to_string()],
            Vec::new(),
            Duration::from_secs(1),
            uid,
            gid,
        )
        .expect("config should validate");

        let error = manager
            .launch_session(&AgentSessionLaunch {
                session_id,
                workspace_id: "w_test",
                checkout: &checkout,
                config: Some(config),
            })
            .expect_err("launch without listening endpoint should fail");

        assert!(
            matches!(error, AgentRuntimeError::Io(message) if message.contains("exited before ACP endpoint"))
        );
        let _ = std::fs::remove_dir_all(state_dir);
        let _ = std::fs::remove_dir_all(cgroup_root);
    }

    #[cfg(target_os = "linux")]
    fn python_acp_server_config() -> AgentLaunchConfig {
        let (uid, gid) = test_agent_identity();
        AgentLaunchConfig::chroot(
            vec![
                python3_path(),
                "-c".to_string(),
                python_acp_server_script(),
                "${ACP_PORT}".to_string(),
            ],
            Vec::new(),
            Duration::from_secs(5),
            uid,
            gid,
        )
        .expect("python ACP server config should validate")
    }

    #[cfg(target_os = "linux")]
    fn python_acp_server_script() -> String {
        "import os,socket,time;\
         s=socket.socket();\
         s.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1);\
         s.bind(('127.0.0.1',int(os.environ['ACP_PORT'])));\
         s.listen(1);\
         time.sleep(60)"
            .to_string()
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn spawn_helpers_cover_error_timeout_and_cleanup_paths() {
        let missing = Command::new("/definitely/missing/acp-agent");
        let cgroup = fake_agent_cgroup("spawn-error");
        let error = spawn_with_timeout(missing, Duration::from_secs(1), Some(cgroup.clone()), None)
            .expect_err("missing binary should fail");
        assert!(
            matches!(error, AgentRuntimeError::Io(message) if message.contains("spawning agent process failed"))
        );

        cleanup_unsent_spawn_result(Ok(spawn_sleep_child()), None);
        assert!(launch_timed_out(Duration::from_millis(1), Some(&cgroup)).is_err());
        let _ = std::fs::remove_dir_all(cgroup.path);
    }

    #[cfg(target_os = "linux")]
    fn fake_agent_cgroup(name: &str) -> AgentCgroup {
        let path = temp_state_dir(&format!("acp-agent-cgroup-{name}"));
        std::fs::create_dir_all(&path).expect("fake cgroup should be creatable");
        std::fs::write(path.join("cgroup.kill"), b"").expect("fake cgroup.kill should write");
        std::fs::write(path.join("cgroup.procs"), b"").expect("fake cgroup.procs should write");
        AgentCgroup { path }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn agent_cgroup_kill_uses_cgroup_kill_when_available() {
        let cgroup_path = std::env::temp_dir().join(format!(
            "acp-agent-cgroup-kill-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&cgroup_path).expect("fake cgroup should be creatable");
        std::fs::write(cgroup_path.join("cgroup.kill"), b"")
            .expect("fake cgroup.kill should be writable");
        let cgroup = AgentCgroup { path: cgroup_path };

        cgroup.kill();

        assert_eq!(
            std::fs::read(cgroup.path.join("cgroup.kill"))
                .expect("fake cgroup.kill should be readable"),
            b"1\n"
        );
        let _ = std::fs::remove_file(cgroup.path.join("cgroup.kill"));
        let _ = std::fs::remove_dir(&cgroup.path);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn chroot_workspace_ownership_is_assigned_to_agent_identity() {
        unsafe {
            if libc::geteuid() != 0 {
                return;
            }
        }
        use std::os::unix::fs::MetadataExt;

        let workspace =
            std::env::temp_dir().join(format!("acp-agent-chown-{}", uuid::Uuid::new_v4().simple()));
        let nested = workspace.join("nested");
        std::fs::create_dir_all(&nested).expect("workspace fixture should be creatable");
        std::fs::write(nested.join("file.txt"), b"data")
            .expect("workspace file should be writable");

        chown_workspace_for_agent(&workspace, DEFAULT_AGENT_RUN_UID, DEFAULT_AGENT_RUN_GID)
            .expect("root should be able to assign workspace ownership");

        let root_metadata =
            std::fs::symlink_metadata(&workspace).expect("workspace metadata should load");
        let file_metadata = std::fs::symlink_metadata(nested.join("file.txt"))
            .expect("workspace file metadata should load");
        assert_eq!(root_metadata.uid(), DEFAULT_AGENT_RUN_UID);
        assert_eq!(root_metadata.gid(), DEFAULT_AGENT_RUN_GID);
        assert_eq!(file_metadata.uid(), DEFAULT_AGENT_RUN_UID);
        assert_eq!(file_metadata.gid(), DEFAULT_AGENT_RUN_GID);
        let _ = std::fs::remove_dir_all(workspace);
    }
}
