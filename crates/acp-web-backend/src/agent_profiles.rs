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
        let mut profile = normalize_profile("profile-pending", request)?;
        profile_to_config(&profile)?;
        let mut profiles = self.lock_profiles()?;
        ensure_unique_profile_name(&profiles, None, &profile.name)?;
        loop {
            let profile_id = format!("profile-{}", uuid::Uuid::new_v4().simple());
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
    let file = AgentProfileFile {
        profiles: profiles.values().cloned().collect(),
    };
    let bytes = serde_json::to_vec_pretty(&file).map_err(|error| {
        AgentProfileStoreError::Json(format!("serializing agent profiles failed: {error}"))
    })?;
    let temp_path = path.with_extension(format!("json.tmp-{}", uuid::Uuid::new_v4().simple()));
    write_profile_bytes(&temp_path, &bytes)?;
    fs::rename(&temp_path, path).map_err(|error| {
        let _ = fs::remove_file(&temp_path);
        AgentProfileStoreError::Io(format!("replacing agent profiles failed: {error}"))
    })
}

fn write_profile_bytes(path: &Path, bytes: &[u8]) -> Result<(), AgentProfileStoreError> {
    let mut file = fs::File::create(path).map_err(|error| {
        AgentProfileStoreError::Io(format!("creating temporary agent profiles failed: {error}"))
    })?;
    file.write_all(bytes).map_err(|error| {
        AgentProfileStoreError::Io(format!("writing temporary agent profiles failed: {error}"))
    })?;
    file.sync_all().map_err(|error| {
        AgentProfileStoreError::Io(format!("syncing temporary agent profiles failed: {error}"))
    })
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
