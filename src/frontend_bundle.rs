use std::{
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

use super::{
    LauncherError, Result,
    support::frontend::{FrontendBundleAsset, is_frontend_bundle_asset},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FrontendBundleState {
    Current,
    Missing,
    Stale,
}

pub(crate) fn frontend_crate_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("crates")
        .join("acp-web-frontend")
}

/// Returns the Trunk dist directory for the web frontend.
/// The path is anchored to the repository root so `cargo run -- --web`
/// keeps working even when launched outside the repo root.
pub(crate) fn frontend_dist_path() -> PathBuf {
    frontend_crate_path().join("dist")
}

/// Ensures the Leptos/WASM frontend bundle exists in `dist`.
/// Runs `trunk build --release` in the frontend crate directory when the
/// compiled artefacts are absent or older than the frontend sources.
pub(crate) async fn ensure_frontend_built(dist: &Path) -> Result<()> {
    ensure_frontend_built_at(&frontend_crate_path(), dist).await
}

pub(crate) async fn ensure_frontend_built_at(frontend_root: &Path, dist: &Path) -> Result<()> {
    match frontend_bundle_state(frontend_root, dist)? {
        FrontendBundleState::Current => return Ok(()),
        FrontendBundleState::Missing => {
            eprintln!(
                "web frontend not built yet – running `trunk build --release` in {} …",
                frontend_root.display()
            );
        }
        FrontendBundleState::Stale => {
            eprintln!(
                "web frontend bundle is stale – running `trunk build --release` in {} …",
                frontend_root.display()
            );
        }
    }

    eprintln!("(re-run with `trunk build --release` in that directory to rebuild manually)");

    let status = tokio::process::Command::new("trunk")
        .args(["build", "--release"])
        .current_dir(frontend_root)
        .status()
        .await
        .map_err(|source| {
            let detail = if source.kind() == std::io::ErrorKind::NotFound {
                "trunk not found – install it with `cargo install trunk`".to_string()
            } else {
                source.to_string()
            };
            LauncherError::FrontendBuild { message: detail }
        })?;

    if !status.success() {
        return Err(LauncherError::FrontendBuild {
            message: format!(
                "`trunk build --release` failed with exit code {:?}",
                status.code()
            ),
        });
    }

    Ok(())
}

pub(crate) fn frontend_bundle_state(
    frontend_root: &Path,
    dist: &Path,
) -> Result<FrontendBundleState> {
    let Some((javascript, wasm)) = frontend_bundle_paths(dist)? else {
        return Ok(FrontendBundleState::Missing);
    };

    let latest_input = latest_frontend_input_modified(frontend_root)?;
    if read_modified_time(&javascript)? < latest_input || read_modified_time(&wasm)? < latest_input
    {
        return Ok(FrontendBundleState::Stale);
    }

    Ok(FrontendBundleState::Current)
}

pub(crate) fn frontend_bundle_paths(dist: &Path) -> Result<Option<(PathBuf, PathBuf)>> {
    let entries = match fs::read_dir(dist) {
        Ok(entries) => entries,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(frontend_build_error("reading", dist, source)),
    };
    let mut javascript = None;
    let mut wasm = None;

    for entry in entries {
        let path = entry
            .map_err(|source| frontend_build_error("reading", dist, source))?
            .path();
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        if is_frontend_bundle_asset(file_name, FrontendBundleAsset::JavaScript) {
            javascript = Some(path);
        } else if is_frontend_bundle_asset(file_name, FrontendBundleAsset::Wasm) {
            wasm = Some(path);
        }
    }

    Ok(match (javascript, wasm) {
        (Some(javascript), Some(wasm)) => Some((javascript, wasm)),
        _ => None,
    })
}

pub(crate) fn latest_frontend_input_modified(frontend_root: &Path) -> Result<SystemTime> {
    let mut latest = None;
    for relative_path in ["Cargo.toml", "Cargo.lock", "Trunk.toml", "index.html"] {
        let path = frontend_root.join(relative_path);
        if path.exists() {
            update_latest_modified(&path, &mut latest)?;
        }
    }
    update_latest_modified(&frontend_root.join("src"), &mut latest)?;

    latest.ok_or_else(|| LauncherError::FrontendBuild {
        message: no_frontend_inputs_message(frontend_root),
    })
}

pub(crate) fn update_latest_modified(path: &Path, latest: &mut Option<SystemTime>) -> Result<()> {
    let metadata =
        fs::metadata(path).map_err(|source| frontend_build_error("reading", path, source))?;

    if metadata.is_dir() {
        let entries =
            fs::read_dir(path).map_err(|source| frontend_build_error("reading", path, source))?;
        for entry in entries {
            let entry = entry.map_err(|source| frontend_build_error("reading", path, source))?;
            update_latest_modified(&entry.path(), latest)?;
        }
        return Ok(());
    }

    let modified = metadata
        .modified()
        .map_err(|source| frontend_build_error("reading modified time for", path, source))?;
    match latest {
        Some(current) if modified <= *current => {}
        _ => *latest = Some(modified),
    }
    Ok(())
}

pub(crate) fn read_modified_time(path: &Path) -> Result<SystemTime> {
    fs::metadata(path)
        .map_err(|source| frontend_build_error("reading", path, source))?
        .modified()
        .map_err(|source| frontend_build_error("reading modified time for", path, source))
}

pub(crate) fn frontend_build_message(action: &str, path: &Path, source: std::io::Error) -> String {
    format!("{action} {} failed: {source}", path.display())
}

pub(crate) fn frontend_build_error(
    action: &str,
    path: &Path,
    source: std::io::Error,
) -> LauncherError {
    LauncherError::FrontendBuild {
        message: frontend_build_message(action, path, source),
    }
}

pub(crate) fn no_frontend_inputs_message(frontend_root: &Path) -> String {
    format!(
        "no frontend inputs were found under {}",
        frontend_root.display()
    )
}
