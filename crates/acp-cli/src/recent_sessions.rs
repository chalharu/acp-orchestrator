use super::*;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct RecentSessionEntry {
    pub(super) session_id: String,
    pub(super) server_url: String,
    pub(super) last_used_at: DateTime<Utc>,
}

impl RecentSessionEntry {
    pub(super) fn new(session_id: &str, server_url: &str, last_used_at: DateTime<Utc>) -> Self {
        Self {
            session_id: session_id.to_string(),
            server_url: server_url.to_string(),
            last_used_at,
        }
    }
}

fn recent_sessions_path() -> Result<PathBuf> {
    recent_sessions_path_from(
        std::env::var_os("ACP_RECENT_SESSIONS_PATH"),
        dirs::data_local_dir(),
    )
}

pub(super) fn recent_sessions_path_from(
    explicit_path: Option<OsString>,
    data_local_dir: Option<PathBuf>,
) -> Result<PathBuf> {
    if let Some(path) = explicit_path {
        return Ok(PathBuf::from(path));
    }

    let mut directory = data_local_dir.ok_or_else(|| MissingRecentSessionDirectorySnafu.build())?;
    directory.push("acp-orchestrator");
    directory.push("recent-sessions.json");
    Ok(directory)
}

pub(super) fn load_recent_sessions() -> Result<Vec<RecentSessionEntry>> {
    let path = recent_sessions_path()?;
    if !path.exists() {
        return Ok(Vec::new());
    }

    let raw = fs::read_to_string(&path).context(ReadRecentSessionsSnafu { path: path.clone() })?;
    let entries = serde_json::from_str(&raw).context(ParseRecentSessionsSnafu { path })?;
    Ok(entries)
}

fn save_recent_sessions(entries: &[RecentSessionEntry]) -> Result<()> {
    let path = recent_sessions_path()?;
    create_recent_sessions_parent(&path)?;

    let serialized = serde_json::to_string_pretty(entries).context(SerializeRecentSessionsSnafu)?;
    fs::write(&path, serialized).context(WriteRecentSessionsSnafu { path })?;
    Ok(())
}

pub(super) fn create_recent_sessions_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        let parent = parent.to_path_buf();
        fs::create_dir_all(&parent).context(CreateRecentSessionsDirectorySnafu { path: parent })?;
    }
    Ok(())
}

pub(super) fn record_recent_session(entry: &RecentSessionEntry) -> Result<()> {
    let mut entries = load_recent_sessions()?;
    entries.retain(|existing| existing.session_id != entry.session_id);
    entries.insert(0, entry.clone());
    entries.truncate(20);
    save_recent_sessions(&entries)
}

pub(super) fn remove_recent_session(session_id: &str) -> Result<()> {
    let mut entries = load_recent_sessions()?;
    entries.retain(|entry| entry.session_id != session_id);
    save_recent_sessions(&entries)
}
