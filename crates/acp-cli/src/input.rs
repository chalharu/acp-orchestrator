use std::io::{self, BufRead, Write};

use reqwest::Client;
use snafu::prelude::*;

use crate::{
    FlushPromptSnafu, JoinPromptReaderSnafu, ReadPromptLineSnafu, Result, api::submit_prompt,
    repl_commands::handle_repl_command,
};

pub(super) async fn drive_repl(
    client: &Client,
    base_url: &str,
    auth_token: &str,
    session_id: &str,
) -> Result<()> {
    drive_line_repl(client, base_url, auth_token, session_id).await
}

async fn drive_line_repl(
    client: &Client,
    base_url: &str,
    auth_token: &str,
    session_id: &str,
) -> Result<()> {
    loop {
        let Some(line) = read_prompt_line().await? else {
            return Ok(());
        };
        if execute_repl_line(client, base_url, auth_token, session_id, &line).await? {
            return Ok(());
        }
    }
}

async fn execute_repl_line(
    client: &Client,
    base_url: &str,
    auth_token: &str,
    session_id: &str,
    line: &str,
) -> Result<bool> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Ok(false);
    }
    if trimmed.starts_with('/') {
        return handle_repl_command(trimmed, client, base_url, auth_token, session_id).await;
    }

    submit_prompt(client, base_url, auth_token, session_id, trimmed).await?;
    Ok(false)
}

async fn read_prompt_line() -> Result<Option<String>> {
    tokio::task::spawn_blocking(|| -> Result<Option<String>> {
        let stdin = io::stdin();
        let stdout = io::stdout();
        let mut stdin = stdin.lock();
        let mut stdout = stdout.lock();
        read_prompt_line_from(&mut stdin, &mut stdout)
    })
    .await
    .context(JoinPromptReaderSnafu)?
}

fn read_prompt_line_from<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
) -> Result<Option<String>> {
    write!(writer, "> ").context(FlushPromptSnafu)?;
    writer.flush().context(FlushPromptSnafu)?;

    let mut buffer = String::new();
    let bytes_read = reader.read_line(&mut buffer).context(ReadPromptLineSnafu)?;

    if bytes_read == 0 {
        Ok(None)
    } else {
        Ok(Some(buffer))
    }
}

#[cfg(test)]
mod tests;
