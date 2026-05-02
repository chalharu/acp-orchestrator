use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};

use acp_contracts_sessions::{AgentProfile, AgentProfileMode, UpsertAgentProfileRequest};
use serde::{Deserialize, Serialize};

use crate::agent_runtime::AgentLaunchConfig;

const PROFILES_FILE: &str = "agent-profiles.json";

#[derive(Debug)]
pub struct AgentProfileStore {
    path: PathBuf,
    profiles: Mutex<BTreeMap<String, AgentProfile>>,
}

#[derive(Debug)]
pub enum AgentProfileStoreError {
    Io(String),
    Json(String),
    Validation(String),
    NotFound,
}

impl AgentProfileStoreError {
    pub fn message(&self) -> &str {
        match self {
            Self::Io(message) | Self::Json(message) | Self::Validation(message) => message,
            Self::NotFound => "agent profile not found",
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct AgentProfileFile {
    #[serde(default)]
    profiles: Vec<AgentProfile>,
}

impl AgentProfileStore {
    pub fn new(state_dir: &Path) -> Result<Self, AgentProfileStoreError> {
        fs::create_dir_all(state_dir).map_err(|error| {
            AgentProfileStoreError::Io(format!("creating profile state directory failed: {error}"))
        })?;
        let path = state_dir.join(PROFILES_FILE);
        let profiles = read_profiles(&path)?;
        Ok(Self {
            path,
            profiles: Mutex::new(profiles),
        })
    }

    pub fn list_profiles(&self) -> Result<Vec<AgentProfile>, AgentProfileStoreError> {
        Ok(self.lock_profiles()?.values().cloned().collect())
    }

    pub fn profile_config(
        &self,
        profile_id: Option<&str>,
    ) -> Result<Option<AgentLaunchConfig>, AgentProfileStoreError> {
        let Some(profile_id) = profile_id else {
            return Ok(None);
        };
        let profile = self
            .lock_profiles()?
            .get(profile_id)
            .cloned()
            .ok_or(AgentProfileStoreError::NotFound)?;
        profile_to_config(&profile).map(Some)
    }

    pub fn upsert_profile(
        &self,
        profile_id: &str,
        request: UpsertAgentProfileRequest,
    ) -> Result<AgentProfile, AgentProfileStoreError> {
        validate_profile_id(profile_id)?;
        let profile = normalize_profile(profile_id, request)?;
        profile_to_config(&profile)?;
        let mut profiles = self.lock_profiles()?;
        ensure_unique_profile_name(&profiles, Some(profile_id), &profile.name)?;
        profiles.insert(profile.id.clone(), profile.clone());
        write_profiles(&self.path, &profiles)?;
        Ok(profile)
    }

    pub fn create_profile(
        &self,
        request: UpsertAgentProfileRequest,
    ) -> Result<AgentProfile, AgentProfileStoreError> {
        self.create_profile_with_id_generator(request, || {
            format!("profile-{}", uuid::Uuid::new_v4().simple())
        })
    }

    pub fn delete_profile(&self, profile_id: &str) -> Result<(), AgentProfileStoreError> {
        validate_profile_id(profile_id)?;
        let mut profiles = self.lock_profiles()?;
        if profiles.remove(profile_id).is_none() {
            return Err(AgentProfileStoreError::NotFound);
        }
        write_profiles(&self.path, &profiles)
    }

    fn create_profile_with_id_generator(
        &self,
        request: UpsertAgentProfileRequest,
        mut next_profile_id: impl FnMut() -> String,
    ) -> Result<AgentProfile, AgentProfileStoreError> {
        let mut profile = normalize_profile("profile-pending", request)?;
        profile_to_config(&profile)?;
        let mut profiles = self.lock_profiles()?;
        ensure_unique_profile_name(&profiles, None, &profile.name)?;
        loop {
            let profile_id = next_profile_id();
            if profiles.contains_key(&profile_id) {
                continue;
            }
            profile.id = profile_id;
            profiles.insert(profile.id.clone(), profile.clone());
            write_profiles(&self.path, &profiles)?;
            return Ok(profile);
        }
    }

    fn lock_profiles(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, BTreeMap<String, AgentProfile>>, AgentProfileStoreError>
    {
        self.profiles.lock().map_err(|_| {
            AgentProfileStoreError::Io("agent profile store lock is poisoned".to_string())
        })
    }
}

fn read_profiles(path: &Path) -> Result<BTreeMap<String, AgentProfile>, AgentProfileStoreError> {
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let bytes = fs::read(path).map_err(|error| {
        AgentProfileStoreError::Io(format!("reading agent profiles failed: {error}"))
    })?;
    let file: AgentProfileFile = serde_json::from_slice(&bytes).map_err(|error| {
        AgentProfileStoreError::Json(format!("parsing agent profiles failed: {error}"))
    })?;
    file.profiles
        .into_iter()
        .map(|profile| {
            validate_stored_profile(&profile)?;
            Ok((profile.id.clone(), profile))
        })
        .collect()
}

fn write_profiles(
    path: &Path,
    profiles: &BTreeMap<String, AgentProfile>,
) -> Result<(), AgentProfileStoreError> {
    write_profiles_with_serializer(path, profiles, serde_json::to_vec_pretty)
}

fn write_profiles_with_serializer(
    path: &Path,
    profiles: &BTreeMap<String, AgentProfile>,
    serialize: impl FnOnce(&AgentProfileFile) -> Result<Vec<u8>, serde_json::Error>,
) -> Result<(), AgentProfileStoreError> {
    let file = AgentProfileFile {
        profiles: profiles.values().cloned().collect(),
    };
    let bytes = serialize(&file).map_err(agent_profile_file_serialization_error)?;
    let temp_path = path.with_extension(format!("json.tmp-{}", uuid::Uuid::new_v4().simple()));
    write_profile_bytes(&temp_path, &bytes)?;
    fs::rename(&temp_path, path).map_err(|error| {
        let _ = fs::remove_file(&temp_path);
        AgentProfileStoreError::Io(format!("replacing agent profiles failed: {error}"))
    })
}

fn agent_profile_file_serialization_error(error: serde_json::Error) -> AgentProfileStoreError {
    AgentProfileStoreError::Json(format!("serializing agent profiles failed: {error}"))
}

fn write_profile_bytes(path: &Path, bytes: &[u8]) -> Result<(), AgentProfileStoreError> {
    let mut file = fs::File::create(path).map_err(|error| {
        AgentProfileStoreError::Io(format!("creating temporary agent profiles failed: {error}"))
    })?;
    file.write_all(bytes).map_err(|error| {
        AgentProfileStoreError::Io(format!("writing temporary agent profiles failed: {error}"))
    })?;
    file.sync_all().map_err(sync_profile_error)
}

fn sync_profile_error(error: std::io::Error) -> AgentProfileStoreError {
    AgentProfileStoreError::Io(format!("syncing temporary agent profiles failed: {error}"))
}

fn normalize_profile(
    profile_id: &str,
    request: UpsertAgentProfileRequest,
) -> Result<AgentProfile, AgentProfileStoreError> {
    let name = request.name.trim().to_string();
    if name.is_empty() {
        return Err(AgentProfileStoreError::Validation(
            "profile name must not be empty".to_string(),
        ));
    }
    let command_argv = normalize_command_argv(request.command_argv)?;
    Ok(AgentProfile {
        id: profile_id.to_string(),
        name,
        mode: request.mode,
        command_argv,
        env_allowlist: request.env_allowlist,
        timeout_seconds: request.timeout_seconds,
        run_uid: request.run_uid,
        run_gid: request.run_gid,
    })
}

fn ensure_unique_profile_name(
    profiles: &BTreeMap<String, AgentProfile>,
    allowed_profile_id: Option<&str>,
    name: &str,
) -> Result<(), AgentProfileStoreError> {
    let duplicate = profiles.iter().any(|(profile_id, profile)| {
        Some(profile_id.as_str()) != allowed_profile_id && profile.name == name
    });
    if duplicate {
        Err(AgentProfileStoreError::Validation(
            "profile name already exists".to_string(),
        ))
    } else {
        Ok(())
    }
}

fn normalize_command_argv(
    command_argv: Vec<String>,
) -> Result<Vec<String>, AgentProfileStoreError> {
    let command_argv: Vec<String> = command_argv
        .into_iter()
        .map(|arg| arg.trim().to_string())
        .filter(|arg| !arg.is_empty())
        .collect();
    if command_argv.is_empty() {
        return Err(AgentProfileStoreError::Validation(
            "profile command must include at least one argv element".to_string(),
        ));
    }
    Ok(command_argv)
}

fn profile_to_config(profile: &AgentProfile) -> Result<AgentLaunchConfig, AgentProfileStoreError> {
    match profile.mode {
        AgentProfileMode::Host => AgentLaunchConfig::host(
            profile.command_argv.clone(),
            profile.env_allowlist.clone(),
            Duration::from_secs(profile.timeout_seconds),
            profile.run_uid,
            profile.run_gid,
        )
        .map_err(|error| AgentProfileStoreError::Validation(error.to_string())),
        AgentProfileMode::Chroot => AgentLaunchConfig::chroot(
            profile.command_argv.clone(),
            profile.env_allowlist.clone(),
            Duration::from_secs(profile.timeout_seconds),
            profile.run_uid,
            profile.run_gid,
        )
        .map_err(|error| AgentProfileStoreError::Validation(error.to_string())),
    }
}

fn validate_stored_profile(profile: &AgentProfile) -> Result<(), AgentProfileStoreError> {
    validate_profile_id(&profile.id)?;
    let request = UpsertAgentProfileRequest {
        name: profile.name.clone(),
        mode: profile.mode.clone(),
        command_argv: profile.command_argv.clone(),
        env_allowlist: profile.env_allowlist.clone(),
        timeout_seconds: profile.timeout_seconds,
        run_uid: profile.run_uid,
        run_gid: profile.run_gid,
    };
    normalize_profile(&profile.id, request)?;
    profile_to_config(profile)?;
    Ok(())
}

fn validate_profile_id(profile_id: &str) -> Result<(), AgentProfileStoreError> {
    let valid = !profile_id.is_empty()
        && profile_id
            .chars()
            .all(|ch| ch == '-' || ch == '_' || ch.is_ascii_alphanumeric());
    if valid {
        Ok(())
    } else {
        Err(AgentProfileStoreError::Validation(
            "profile id is invalid".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_runtime::{
        DEFAULT_AGENT_LAUNCH_TIMEOUT, DEFAULT_AGENT_RUN_GID, DEFAULT_AGENT_RUN_UID,
    };

    fn request(command_argv: Vec<String>) -> UpsertAgentProfileRequest {
        UpsertAgentProfileRequest {
            name: "OpenCode".to_string(),
            mode: AgentProfileMode::Chroot,
            command_argv,
            env_allowlist: Vec::new(),
            timeout_seconds: DEFAULT_AGENT_LAUNCH_TIMEOUT.as_secs(),
            run_uid: DEFAULT_AGENT_RUN_UID,
            run_gid: DEFAULT_AGENT_RUN_GID,
        }
    }

    #[test]
    fn profile_store_round_trips_valid_profiles() {
        let state_dir = std::env::temp_dir().join(format!(
            "acp-profile-store-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let store = AgentProfileStore::new(&state_dir).expect("store should initialize");
        let profile = store
            .upsert_profile(
                "opencode",
                request(vec!["opencode".to_string(), "acp".to_string()]),
            )
            .expect("profile should save");

        assert_eq!(profile.id, "opencode");
        assert_eq!(store.list_profiles().expect("list"), vec![profile]);
    }

    #[test]
    fn profile_store_deletes_existing_profiles_and_reports_missing() {
        let state_dir = std::env::temp_dir().join(format!(
            "acp-profile-store-delete-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let store = AgentProfileStore::new(&state_dir).expect("store should initialize");
        store
            .upsert_profile(
                "opencode",
                request(vec!["opencode".to_string(), "acp".to_string()]),
            )
            .expect("profile should save");

        store
            .delete_profile("opencode")
            .expect("profile should delete");
        assert!(store.list_profiles().expect("list").is_empty());
        assert!(matches!(
            store
                .delete_profile("opencode")
                .expect_err("missing profile should fail"),
            AgentProfileStoreError::NotFound
        ));
    }

    #[test]
    fn profile_store_loads_existing_profiles_and_builds_runtime_config() {
        let state_dir = std::env::temp_dir().join(format!(
            "acp-profile-store-load-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let store = AgentProfileStore::new(&state_dir).expect("store should initialize");
        let profile = store
            .upsert_profile(
                "opencode",
                request(vec!["opencode".to_string(), "acp".to_string()]),
            )
            .expect("profile should save");
        drop(store);

        let reloaded = AgentProfileStore::new(&state_dir).expect("store should reload");
        let config = reloaded
            .profile_config(Some("opencode"))
            .expect("profile config should load")
            .expect("profile config should exist");

        assert_eq!(reloaded.list_profiles().expect("list"), vec![profile]);
        assert_eq!(config.command, vec!["opencode", "acp"]);
    }

    #[test]
    fn profile_store_builds_host_runtime_configs() {
        let state_dir = std::env::temp_dir().join(format!(
            "acp-profile-store-host-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let store = AgentProfileStore::new(&state_dir).expect("store should initialize");
        let mut host = request(vec!["opencode".to_string(), "acp".to_string()]);
        host.mode = AgentProfileMode::Host;
        host.run_uid = 0;
        host.run_gid = 0;
        store
            .upsert_profile("opencode", host)
            .expect("host profile should save");

        let config = store
            .profile_config(Some("opencode"))
            .expect("profile config should load")
            .expect("profile config should exist");

        assert_eq!(config.mode, crate::agent_runtime::AgentLaunchMode::Host);
        assert_eq!(config.run_uid, 0);
        assert_eq!(config.run_gid, 0);
    }

    #[test]
    fn profile_store_returns_none_without_profile_selection() {
        let state_dir = std::env::temp_dir().join(format!(
            "acp-profile-store-none-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let store = AgentProfileStore::new(&state_dir).expect("store should initialize");

        assert!(store.profile_config(None).expect("none config").is_none());
    }

    #[test]
    fn profile_store_reports_missing_profiles() {
        let state_dir = std::env::temp_dir().join(format!(
            "acp-profile-store-missing-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let store = AgentProfileStore::new(&state_dir).expect("store should initialize");
        let error = store
            .profile_config(Some("missing"))
            .expect_err("missing profile should fail");

        assert_eq!(error.message(), "agent profile not found");
    }

    #[test]
    fn profile_store_rejects_blank_profile_names() {
        let state_dir = std::env::temp_dir().join(format!(
            "acp-profile-store-blank-name-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let store = AgentProfileStore::new(&state_dir).expect("store should initialize");
        let mut invalid = request(vec!["opencode".to_string()]);
        invalid.name = "   ".to_string();
        let error = store
            .create_profile(invalid)
            .expect_err("blank profile name should fail");

        assert!(matches!(
            error,
            AgentProfileStoreError::Validation(message)
                if message == "profile name must not be empty"
        ));
    }

    #[test]
    fn profile_store_rejects_invalid_profile_ids() {
        let state_dir = std::env::temp_dir().join(format!(
            "acp-profile-store-bad-id-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let store = AgentProfileStore::new(&state_dir).expect("store should initialize");
        let error = store
            .upsert_profile("bad/id", request(vec!["opencode".to_string()]))
            .expect_err("invalid ids should fail");

        assert!(matches!(
            error,
            AgentProfileStoreError::Validation(message) if message == "profile id is invalid"
        ));
    }

    #[test]
    fn profile_store_rejects_corrupt_profile_files() {
        let state_dir = std::env::temp_dir().join(format!(
            "acp-profile-store-corrupt-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&state_dir).expect("state dir should be creatable");
        std::fs::write(state_dir.join(PROFILES_FILE), b"not-json")
            .expect("corrupt profile fixture should be writable");
        let error =
            AgentProfileStore::new(&state_dir).expect_err("corrupt profile file should fail");

        assert!(
            matches!(error, AgentProfileStoreError::Json(message) if message.contains("parsing agent profiles failed"))
        );
    }

    #[test]
    fn profile_store_reports_state_directory_creation_failures() {
        let file_path = std::env::temp_dir().join(format!(
            "acp-profile-store-file-parent-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::write(&file_path, b"not a directory").expect("file fixture should be writable");
        let error = AgentProfileStore::new(&file_path.join("child"))
            .expect_err("file parent should fail directory creation");

        assert!(
            matches!(error, AgentProfileStoreError::Io(message) if message.contains("creating profile state directory failed"))
        );
        let _ = std::fs::remove_file(file_path);
    }

    #[test]
    fn profile_store_reports_profile_read_failures() {
        let state_dir = std::env::temp_dir().join(format!(
            "acp-profile-store-read-error-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(state_dir.join(PROFILES_FILE))
            .expect("directory fixture should be creatable");
        let error = AgentProfileStore::new(&state_dir).expect_err("directory read should fail");

        assert!(
            matches!(error, AgentProfileStoreError::Io(message) if message.contains("reading agent profiles failed"))
        );
        let _ = std::fs::remove_dir_all(state_dir);
    }

    #[test]
    fn profile_store_reports_temporary_file_creation_failures() {
        let path = std::env::temp_dir()
            .join(format!(
                "acp-profile-missing-dir-{}",
                uuid::Uuid::new_v4().simple()
            ))
            .join(PROFILES_FILE);
        let profiles = BTreeMap::new();
        let error =
            write_profiles(&path, &profiles).expect_err("missing parent should fail temp create");

        assert!(
            matches!(error, AgentProfileStoreError::Io(message) if message.contains("creating temporary agent profiles failed"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn profile_store_reports_profile_write_failures() {
        let error = write_profile_bytes(Path::new("/dev/full"), b"profile")
            .expect_err("/dev/full should reject writes");

        assert!(
            matches!(error, AgentProfileStoreError::Io(message) if message.contains("writing temporary agent profiles failed"))
        );
    }

    #[test]
    fn profile_store_formats_profile_sync_failures() {
        let error = sync_profile_error(std::io::Error::other("sync failed"));

        assert!(
            matches!(error, AgentProfileStoreError::Io(message) if message == "syncing temporary agent profiles failed: sync failed")
        );
    }

    #[test]
    fn profile_store_reports_profile_serialization_failures() {
        let path = std::env::temp_dir().join(format!(
            "acp-profile-serialize-error-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let profiles = BTreeMap::new();
        let error = write_profiles_with_serializer(&path, &profiles, |_| {
            Err(serde_json::from_str::<serde_json::Value>("{").expect_err("invalid JSON"))
        })
        .expect_err("serialization failures should propagate");

        assert!(
            matches!(error, AgentProfileStoreError::Json(message) if message.contains("serializing agent profiles failed"))
        );
    }

    #[test]
    fn profile_store_reports_replace_failures() {
        let state_dir = std::env::temp_dir().join(format!(
            "acp-profile-store-replace-error-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let path = state_dir.join(PROFILES_FILE);
        std::fs::create_dir_all(&path).expect("directory target should be creatable");
        let profiles = BTreeMap::new();
        let error = write_profiles(&path, &profiles).expect_err("directory replace should fail");

        assert!(
            matches!(error, AgentProfileStoreError::Io(message) if message.contains("replacing agent profiles failed"))
        );
        let _ = std::fs::remove_dir_all(state_dir);
    }

    #[test]
    fn profile_store_reports_poisoned_locks() {
        let state_dir = std::env::temp_dir().join(format!(
            "acp-profile-store-poison-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let store = std::sync::Arc::new(AgentProfileStore::new(&state_dir).expect("store"));
        let poisoned = store.clone();
        let _ = std::thread::spawn(move || {
            let _guard = poisoned.profiles.lock().expect("lock should succeed");
            panic!("poison profile store");
        })
        .join();

        let error = store
            .list_profiles()
            .expect_err("poisoned lock should fail");

        assert!(
            matches!(error, AgentProfileStoreError::Io(message) if message == "agent profile store lock is poisoned")
        );
    }

    #[test]
    fn profile_store_rejects_invalid_stored_profiles() {
        let state_dir = std::env::temp_dir().join(format!(
            "acp-profile-store-invalid-file-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&state_dir).expect("state dir should be creatable");
        std::fs::write(
            state_dir.join(PROFILES_FILE),
            r#"{"profiles":[{"id":"bad/id","name":"Bad","mode":"chroot","command_argv":["agent"],"env_allowlist":[],"timeout_seconds":30,"run_uid":65534,"run_gid":65534}]}"#,
        )
        .expect("invalid profile fixture should be writable");
        let error =
            AgentProfileStore::new(&state_dir).expect_err("invalid stored profile should fail");

        assert!(matches!(
            error,
            AgentProfileStoreError::Validation(message) if message == "profile id is invalid"
        ));
    }

    #[test]
    fn profile_store_rejects_invalid_command() {
        let state_dir = std::env::temp_dir().join(format!(
            "acp-profile-store-invalid-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let store = AgentProfileStore::new(&state_dir).expect("store should initialize");
        let error = store
            .upsert_profile("opencode", request(vec![" ".to_string()]))
            .expect_err("blank command should fail");

        assert!(matches!(error, AgentProfileStoreError::Validation(_)));
    }

    #[test]
    fn profile_store_rejects_invalid_runtime_fields() {
        let state_dir = std::env::temp_dir().join(format!(
            "acp-profile-store-runtime-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let store = AgentProfileStore::new(&state_dir).expect("store should initialize");
        let mut invalid = request(vec!["opencode".to_string()]);
        invalid.timeout_seconds = 0;

        let error = store
            .upsert_profile("opencode", invalid)
            .expect_err("zero timeout should fail");

        assert!(matches!(error, AgentProfileStoreError::Validation(_)));
    }

    #[test]
    fn profile_store_creates_generated_profile_ids_for_arbitrary_names() {
        let state_dir = std::env::temp_dir().join(format!(
            "acp-profile-store-create-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let store = AgentProfileStore::new(&state_dir).expect("store should initialize");
        let mut request = request(vec!["claude".to_string(), "acp".to_string()]);
        request.name = "Claude ACP".to_string();

        let profile = store.create_profile(request).expect("profile should save");

        assert!(profile.id.starts_with("profile-"));
        assert_eq!(profile.name, "Claude ACP");
        assert_eq!(profile.command_argv, vec!["claude", "acp"]);
    }

    #[test]
    fn profile_store_retries_generated_profile_id_collisions() {
        let state_dir = std::env::temp_dir().join(format!(
            "acp-profile-store-create-collision-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let store = AgentProfileStore::new(&state_dir).expect("store should initialize");
        store
            .upsert_profile("profile-collision", request(vec!["opencode".to_string()]))
            .expect("collision fixture should save");
        let mut ids = ["profile-collision", "profile-created"].into_iter();
        let mut request = request(vec!["claude".to_string(), "acp".to_string()]);
        request.name = "Claude ACP".to_string();

        let profile = store
            .create_profile_with_id_generator(request, || ids.next().expect("next id").to_string())
            .expect("profile should save after collision");

        assert_eq!(profile.id, "profile-created");
    }

    #[test]
    fn profile_store_rejects_duplicate_profile_names() {
        let state_dir = std::env::temp_dir().join(format!(
            "acp-profile-store-duplicate-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let store = AgentProfileStore::new(&state_dir).expect("store should initialize");
        let mut base_request = request(vec!["claude".to_string(), "acp".to_string()]);
        base_request.name = "Claude ACP".to_string();
        store
            .upsert_profile("claude", base_request.clone())
            .expect("first profile should save");

        let mut duplicate_create = request(vec!["copilot".to_string(), "acp".to_string()]);
        duplicate_create.name = "Claude ACP".to_string();
        let create_error = store
            .create_profile(duplicate_create)
            .expect_err("duplicate create should fail");
        assert!(matches!(
            create_error,
            AgentProfileStoreError::Validation(message) if message == "profile name already exists"
        ));

        let mut renamed = base_request;
        renamed.command_argv = vec![
            "claude".to_string(),
            "acp".to_string(),
            "--verbose".to_string(),
        ];
        store
            .upsert_profile("claude", renamed)
            .expect("same id can update the existing profile name");

        let mut duplicate_upsert = request(vec!["claude".to_string(), "acp".to_string()]);
        duplicate_upsert.name = "Claude ACP".to_string();
        let upsert_error = store
            .upsert_profile("claude-copy", duplicate_upsert)
            .expect_err("duplicate upsert should fail");
        assert!(matches!(
            upsert_error,
            AgentProfileStoreError::Validation(message) if message == "profile name already exists"
        ));
    }
}
