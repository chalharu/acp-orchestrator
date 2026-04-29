use std::{
    collections::HashMap,
    env,
    ffi::CString,
    fmt::Display,
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Condvar, Mutex},
    time::Duration,
};

use crate::workspace_checkout::PreparedWorkspaceCheckout;

pub const AGENT_RUNTIMES_DIR_NAME: &str = "agent-runtimes";
pub const CHROOT_CHECKOUT_ROOT: &str = "/workspace";
pub const DEFAULT_AGENT_RUN_UID: u32 = 65_534;
pub const DEFAULT_AGENT_RUN_GID: u32 = 65_534;
pub const DEFAULT_AGENT_LAUNCH_TIMEOUT: Duration = Duration::from_secs(30);
#[cfg(target_os = "linux")]
const DEFAULT_AGENT_CGROUP_ROOT: &str = "/sys/fs/cgroup/acp-orchestrator";

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
            .find(|name| !is_safe_env_name(name))
        {
            return Err(AgentLaunchConfigError::InvalidEnvName(name.clone()));
        }
        Ok(())
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
}

pub trait AgentRuntimeManager: Send + Sync + std::fmt::Debug {
    fn launch_session(&self, launch: &AgentSessionLaunch<'_>) -> Result<(), AgentRuntimeError>;
    fn forget_session(&self, session_id: &str);
}

pub type DynAgentRuntimeManager = Arc<dyn AgentRuntimeManager>;

#[derive(Debug, Default)]
pub struct NoopAgentRuntimeManager;

impl AgentRuntimeManager for NoopAgentRuntimeManager {
    fn launch_session(&self, _launch: &AgentSessionLaunch<'_>) -> Result<(), AgentRuntimeError> {
        Ok(())
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
}

#[derive(Debug, Clone)]
struct AgentCgroup {
    path: PathBuf,
}

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
    ) -> Result<AgentChild, AgentRuntimeError> {
        let root_dir = self.root_dir(launch.session_id);
        fs::create_dir_all(&root_dir).map_err(|error| {
            AgentRuntimeError::Io(format!("creating agent chroot root failed: {error}"))
        })?;
        chown_workspace_for_agent(&launch.checkout.working_dir, config.run_uid, config.run_gid)?;
        let cgroup = self.prepare_session_cgroup(launch.session_id)?;

        let mut command = Command::new(&config.command[0]);
        command.args(&config.command[1..]);
        command.env_clear();
        for name in &config.env_allowlist {
            if let Some(value) = env::var_os(name) {
                command.env(name, value);
            }
        }
        command.env("ACP_SESSION_ID", launch.session_id);
        command.env("ACP_WORKSPACE_ID", launch.workspace_id);
        command.env("ACP_CHECKOUT_ROOT", CHROOT_CHECKOUT_ROOT);
        command.env("ACP_CHECKOUT_RELPATH", "workspace");
        command.env("ACP_AGENT_LAUNCH_MODE", AgentLaunchMode::Chroot.as_str());
        command.stdin(Stdio::null());
        command.stdout(Stdio::null());
        command.stderr(Stdio::null());

        configure_chroot_command(
            &mut command,
            &root_dir,
            &cgroup,
            config.run_uid,
            config.run_gid,
        )?;
        spawn_with_timeout(command, config.timeout, Some(cgroup))
    }

    fn prepare_session_cgroup(&self, session_id: &str) -> Result<AgentCgroup, AgentRuntimeError> {
        let Some(root) = self.cgroup_root.as_ref() else {
            return Err(AgentRuntimeError::Unsupported(
                "chroot agent launch requires a Linux cgroup v2 root".to_string(),
            ));
        };
        AgentCgroup::prepare(root, session_id)
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
        match registry.slots.get_mut(session_id) {
            Some(slot @ AgentChildSlot::Launching(existing_id)) if *existing_id == launch_id => {
                *slot = AgentChildSlot::Running(child);
                self.launch_notifications.notify_all();
                Ok(())
            }
            Some(AgentChildSlot::Launching(_)) | None => {
                terminate_agent_child(&mut child);
                Err(AgentRuntimeError::Io(
                    "agent runtime launch was cancelled".to_string(),
                ))
            }
            Some(AgentChildSlot::Running(_)) => {
                terminate_agent_child(&mut child);
                Err(AgentRuntimeError::AlreadyRunning(session_id.to_string()))
            }
        }
    }
}

impl AgentRuntimeManager for FsAgentRuntimeManager {
    fn launch_session(&self, launch: &AgentSessionLaunch<'_>) -> Result<(), AgentRuntimeError> {
        let Some(config) = self.config.as_ref() else {
            return Ok(());
        };
        config.validate()?;
        let launch_id = self.reserve_launch(launch.session_id)?;
        let child = match config.mode {
            AgentLaunchMode::Chroot => self.launch_chroot(config, launch),
        };
        let child = match child {
            Ok(child) => child,
            Err(error) => {
                self.clear_launch_reservation(launch.session_id, launch_id);
                return Err(error);
            }
        };
        self.store_launched_child(launch.session_id, launch_id, child)
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

fn is_safe_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

#[cfg(target_os = "linux")]
fn default_agent_cgroup_root() -> Option<PathBuf> {
    Some(PathBuf::from(DEFAULT_AGENT_CGROUP_ROOT))
}

#[cfg(not(target_os = "linux"))]
fn default_agent_cgroup_root() -> Option<PathBuf> {
    None
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

#[cfg(not(target_os = "linux"))]
impl AgentCgroup {
    fn prepare(_root: &Path, _session_id: &str) -> Result<Self, AgentRuntimeError> {
        Err(AgentRuntimeError::Unsupported(
            "chroot agent launch requires Linux cgroup v2".to_string(),
        ))
    }

    fn kill(&self) {}

    fn remove(&self) {}
}

#[cfg(target_os = "linux")]
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

#[cfg(target_os = "linux")]
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

#[cfg(not(target_os = "linux"))]
fn chown_workspace_for_agent(_path: &Path, _uid: u32, _gid: u32) -> Result<(), AgentRuntimeError> {
    Err(AgentRuntimeError::Unsupported(
        "chroot agent launch is only supported on Linux".to_string(),
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

    let root = CString::new(root_dir.as_os_str().as_bytes()).map_err(|_| {
        AgentRuntimeError::Io("agent chroot root path contains a NUL byte".to_string())
    })?;
    let workspace =
        CString::new(CHROOT_CHECKOUT_ROOT).expect("static chroot workspace path has no NUL");
    let cgroup_procs = cgroup.procs_cstring()?;

    unsafe {
        command.pre_exec(move || {
            move_current_process_to_cgroup(&cgroup_procs)?;
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::chroot(root.as_ptr()) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::chdir(workspace.as_ptr()) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::setgroups(0, std::ptr::null()) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::setgid(run_gid) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::setuid(run_uid) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn configure_chroot_command(
    _command: &mut Command,
    _root_dir: &Path,
    _cgroup: &AgentCgroup,
    _run_uid: u32,
    _run_gid: u32,
) -> Result<(), AgentRuntimeError> {
    Err(AgentRuntimeError::Unsupported(
        "chroot agent launch is only supported on Linux".to_string(),
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
) -> Result<AgentChild, AgentRuntimeError> {
    let (sender, receiver) = std::sync::mpsc::sync_channel(0);
    let cgroup_for_timeout = cgroup.clone();
    std::thread::spawn(move || {
        let spawn_result = match command.spawn() {
            Ok(child) => Ok(AgentChild {
                child,
                cgroup: cgroup.clone(),
            }),
            Err(error) => {
                if let Some(cgroup) = cgroup.as_ref() {
                    cgroup.remove();
                }
                Err(error)
            }
        };
        if let Err(std::sync::mpsc::SendError(result)) = sender.send(spawn_result) {
            match result {
                Ok(mut child) => terminate_agent_child(&mut child),
                Err(_) => {
                    if let Some(cgroup) = cgroup.as_ref() {
                        cgroup.remove();
                    }
                }
            }
        }
    });
    match receiver.recv_timeout(timeout) {
        Ok(result) => result.map_err(|error| {
            AgentRuntimeError::Io(format!("spawning agent process failed: {error}"))
        }),
        Err(_) => {
            if let Some(cgroup) = cgroup_for_timeout.as_ref() {
                cgroup.kill();
            }
            Err(AgentRuntimeError::LaunchTimedOut(timeout))
        }
    }
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

    #[test]
    fn launch_config_validation_rejects_unsafe_values() {
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
            })
            .expect("noop launch should succeed");
        manager.forget_session("s_test");
    }

    #[test]
    fn runtime_forget_removes_runtime_directory_without_launching() {
        let state_dir = std::env::temp_dir().join(format!(
            "acp-agent-runtime-cleanup-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let runtime_dir = FsAgentRuntimeManager::runtime_dir_for_state(&state_dir, "s_test");
        std::fs::create_dir_all(runtime_dir.join("root/workspace"))
            .expect("runtime fixture should be creatable");
        let manager = FsAgentRuntimeManager::new(state_dir, None)
            .expect("manager without launch config should build");

        manager.forget_session("s_test");

        assert!(!runtime_dir.exists(), "forget should remove runtime roots");
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
                }),
            );

        manager.forget_session("s_test");

        assert!(!process_is_running(pid), "forget should terminate children");
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
