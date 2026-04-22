use std::{
    fs,
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
    process,
    time::Duration,
};

use snafu::ResultExt;

use crate::Result;

#[derive(Debug)]
pub(crate) struct LauncherLock {
    pub(super) path: PathBuf,
}

impl Drop for LauncherLock {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_file(&self.path)
            && error.kind() != ErrorKind::NotFound
        {
            warn_lock_cleanup_failure(&self.path, &error);
        }
    }
}

pub(crate) fn launcher_lock_path_from(state_path: &Path) -> PathBuf {
    let mut path = state_path.as_os_str().to_os_string();
    path.push(".lock");
    PathBuf::from(path)
}

pub(crate) fn try_acquire_launcher_lock(lock_path: &Path) -> Result<Option<LauncherLock>> {
    super::create_launcher_state_parent(lock_path)?;
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(lock_path)
    {
        Ok(mut file) => write_launcher_lock_owner(&mut file, lock_path).map(Some),
        Err(error) if error.kind() == ErrorKind::AlreadyExists => Ok(None),
        Err(source) => Err(crate::LauncherError::AcquireLauncherLock {
            source,
            path: lock_path.to_path_buf(),
        }),
    }
}

pub(crate) fn clear_stale_launcher_lock(lock_path: &Path, stale_after: Duration) -> Result<bool> {
    let metadata = match fs::metadata(lock_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(source) => {
            return Err(crate::LauncherError::ReadLauncherLockMetadata {
                source,
                path: lock_path.to_path_buf(),
            });
        }
    };
    if metadata.is_file()
        && launcher_lock_owner_pid(lock_path)
            .is_some_and(|owner_pid| !launcher_lock_owner_is_running(owner_pid))
    {
        return remove_launcher_lock(lock_path);
    }

    let modified = metadata
        .modified()
        .context(crate::ReadLauncherLockMetadataSnafu {
            path: lock_path.to_path_buf(),
        })?;
    let is_stale = modified
        .elapsed()
        .ok()
        .is_some_and(|elapsed| elapsed >= stale_after);
    if !is_stale {
        return Ok(false);
    }

    remove_launcher_lock(lock_path)
}

fn warn_lock_cleanup_failure(path: &Path, error: &std::io::Error) {
    tracing::warn!(path = %path.display(), %error, "failed to remove the launcher lock file");
}

fn launcher_lock_owner_pid(lock_path: &Path) -> Option<u32> {
    fs::read_to_string(lock_path)
        .ok()?
        .trim()
        .parse::<u32>()
        .ok()
        .filter(|owner_pid| *owner_pid != 0)
}

pub(crate) fn write_launcher_lock_owner<W: Write>(
    writer: &mut W,
    lock_path: &Path,
) -> Result<LauncherLock> {
    if let Err(source) = writer.write_all(process::id().to_string().as_bytes()) {
        let _ = fs::remove_file(lock_path);
        return Err(crate::LauncherError::AcquireLauncherLock {
            source,
            path: lock_path.to_path_buf(),
        });
    }

    Ok(LauncherLock {
        path: lock_path.to_path_buf(),
    })
}

#[cfg(unix)]
fn launcher_lock_owner_is_running(owner_pid: u32) -> bool {
    let result = unsafe { libc::kill(owner_pid as libc::pid_t, 0) };
    if result == 0 {
        return true;
    }

    std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}

#[cfg(not(unix))]
fn launcher_lock_owner_is_running(_owner_pid: u32) -> bool {
    true
}

fn remove_launcher_lock(lock_path: &Path) -> Result<bool> {
    match fs::remove_file(lock_path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(source) => Err(crate::LauncherError::RemoveLauncherLock {
            source,
            path: lock_path.to_path_buf(),
        }),
    }
}
