use std::{
    ffi::OsString,
    path::Path,
    process::{ExitStatus, Stdio},
    time::Duration,
};

use snafu::prelude::*;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::{Child, Command},
};

use crate::{
    CheckChildStatusSnafu, ChildExitSnafu, InvalidStartupLineSnafu, MissingChildStdoutSnafu,
    ReadStartupLineSnafu, Result, SpawnChildSnafu, TerminateChildSnafu, WaitForChildSnafu,
};

const STARTUP_LINE_TIMEOUT: Duration = Duration::from_secs(35);

pub(crate) struct SpawnedService {
    pub(crate) child: Child,
    pub(crate) endpoint: String,
}

pub(crate) async fn spawn_background_role(
    current_executable: &Path,
    role_label: &'static str,
    role_name: &'static str,
    role_args: Vec<OsString>,
    envs: &[(&str, &str)],
    shutdown_on_drop: bool,
) -> Result<SpawnedService> {
    let mut command = role_command(current_executable, role_name, role_args, envs);
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::inherit());
    command.kill_on_drop(shutdown_on_drop);
    let mut child = command
        .spawn()
        .context(SpawnChildSnafu { role: role_label })?;
    let endpoint = read_startup_url(&mut child, role_label).await?;
    Ok(SpawnedService { child, endpoint })
}

pub(crate) async fn spawn_foreground_role(
    current_executable: &Path,
    role_label: &'static str,
    role_name: &'static str,
    role_args: Vec<OsString>,
    envs: &[(&str, &str)],
) -> Result<ExitStatus> {
    let mut child = role_command(current_executable, role_name, role_args, envs)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .context(SpawnChildSnafu { role: role_label })?;

    child
        .wait()
        .await
        .context(WaitForChildSnafu { role: role_label })
}

fn role_command(
    current_executable: &Path,
    role_name: &'static str,
    role_args: Vec<OsString>,
    envs: &[(&str, &str)],
) -> Command {
    let mut command = Command::new(current_executable);
    command.arg("__internal-role").arg(role_name);

    for arg in role_args {
        command.arg(arg);
    }

    for (key, value) in envs {
        command.env(key, value);
    }

    command
}

pub(crate) async fn read_startup_url(child: &mut Child, role: &'static str) -> Result<String> {
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| MissingChildStdoutSnafu { role }.build())?;
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    let bytes_read = tokio::time::timeout(STARTUP_LINE_TIMEOUT, reader.read_line(&mut line))
        .await
        .map_err(|_| crate::LauncherError::WaitForStartupLine { role })?
        .context(ReadStartupLineSnafu { role })?;

    if bytes_read == 0 {
        return InvalidStartupLineSnafu {
            role,
            line: "<empty>".to_string(),
        }
        .fail();
    }

    let prefix = "listening on ";
    let line = line.trim();
    let base_url = line
        .split_once(prefix)
        .map(|(_, value)| value.to_string())
        .ok_or_else(|| {
            InvalidStartupLineSnafu {
                role,
                line: line.to_string(),
            }
            .build()
        })?;

    Ok(base_url)
}

pub(crate) async fn terminate_child(child: &mut Child, role: &'static str) -> Result<()> {
    if child
        .try_wait()
        .context(CheckChildStatusSnafu { role })?
        .is_some()
    {
        return Ok(());
    }

    child.kill().await.context(TerminateChildSnafu { role })?;
    let _ = child.wait().await.context(WaitForChildSnafu { role })?;
    Ok(())
}

pub(crate) fn ensure_success(role: &'static str, status: ExitStatus) -> Result<()> {
    if status.success() {
        Ok(())
    } else {
        ChildExitSnafu {
            role,
            code: status.code(),
        }
        .fail()
    }
}
