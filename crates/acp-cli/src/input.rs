use std::{
    io::{self, BufRead, IsTerminal, Write},
    time::Duration,
};

use acp_contracts::{CompletionCandidate, CompletionKind, classify_slash_completion_prefix};
use reqwest::Client;
use rustyline::{
    Context, Editor, Helper,
    completion::{Completer, Pair},
    error::ReadlineError,
    highlight::Highlighter,
    hint::Hinter,
    history::DefaultHistory,
    validate::Validator,
};
use snafu::prelude::*;
use tokio::runtime::Handle;

use crate::{
    BuildInteractiveEditorSnafu, FlushPromptSnafu, JoinPromptReaderSnafu, ReadPromptLineSnafu,
    Result,
    api::{get_slash_completions, submit_prompt},
    repl_commands::handle_repl_command,
};

const SLASH_COMPLETION_TIMEOUT: Duration = Duration::from_secs(2);

pub(super) fn interactive_completion_enabled() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

pub(super) async fn drive_repl(
    client: &Client,
    base_url: &str,
    auth_token: &str,
    session_id: &str,
) -> Result<()> {
    if interactive_completion_enabled() {
        return drive_interactive_repl(client, base_url, auth_token, session_id).await;
    }

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

async fn drive_interactive_repl(
    client: &Client,
    base_url: &str,
    auth_token: &str,
    session_id: &str,
) -> Result<()> {
    let runtime_handle = Handle::current();
    let client = client.clone();
    let base_url = base_url.to_string();
    let auth_token = auth_token.to_string();
    let session_id = session_id.to_string();

    tokio::task::spawn_blocking(move || {
        interactive_repl(runtime_handle, client, base_url, auth_token, session_id)
    })
    .await
    .context(JoinPromptReaderSnafu)?
}

fn interactive_repl(
    runtime_handle: Handle,
    client: Client,
    base_url: String,
    auth_token: String,
    session_id: String,
) -> Result<()> {
    let helper = SlashCompletionHelper::new(
        runtime_handle.clone(),
        client.clone(),
        base_url.clone(),
        auth_token.clone(),
        session_id.clone(),
    );
    let mut editor = Editor::<SlashCompletionHelper, DefaultHistory>::new()
        .context(BuildInteractiveEditorSnafu)?;
    editor.set_helper(Some(helper));

    drive_editor_repl(
        &mut editor,
        &runtime_handle,
        &client,
        &base_url,
        &auth_token,
        &session_id,
    )
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

trait PromptEditor {
    fn readline(&mut self, prompt: &str) -> std::result::Result<String, ReadlineError>;
    fn add_history_entry(&mut self, line: &str);
}

impl PromptEditor for Editor<SlashCompletionHelper, DefaultHistory> {
    fn readline(&mut self, prompt: &str) -> std::result::Result<String, ReadlineError> {
        Editor::readline(self, prompt)
    }

    fn add_history_entry(&mut self, line: &str) {
        let _ = Editor::add_history_entry(self, line);
    }
}

fn drive_editor_repl<E: PromptEditor>(
    editor: &mut E,
    runtime_handle: &Handle,
    client: &Client,
    base_url: &str,
    auth_token: &str,
    session_id: &str,
) -> Result<()> {
    loop {
        match editor.readline("> ") {
            Ok(line) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                editor.add_history_entry(trimmed);
                if runtime_handle.block_on(execute_repl_line(
                    client, base_url, auth_token, session_id, trimmed,
                ))? {
                    return Ok(());
                }
            }
            Err(ReadlineError::Eof) => return Ok(()),
            Err(ReadlineError::Interrupted) => {
                println!("[status] interrupted input. Use `/quit` to leave the chat.");
            }
            Err(source) => return Err(crate::CliError::ReadInteractivePrompt { source }),
        }
    }
}

#[derive(Clone)]
struct SlashCompletionHelper {
    runtime_handle: Handle,
    client: Client,
    base_url: String,
    auth_token: String,
    session_id: String,
    completion_timeout: Duration,
}

impl SlashCompletionHelper {
    fn new(
        runtime_handle: Handle,
        client: Client,
        base_url: String,
        auth_token: String,
        session_id: String,
    ) -> Self {
        Self::with_timeout(
            runtime_handle,
            client,
            base_url,
            auth_token,
            session_id,
            SLASH_COMPLETION_TIMEOUT,
        )
    }

    fn with_timeout(
        runtime_handle: Handle,
        client: Client,
        base_url: String,
        auth_token: String,
        session_id: String,
        completion_timeout: Duration,
    ) -> Self {
        Self {
            runtime_handle,
            client,
            base_url,
            auth_token,
            session_id,
            completion_timeout,
        }
    }
}

impl Helper for SlashCompletionHelper {}

impl Completer for SlashCompletionHelper {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        position: usize,
        _context: &Context<'_>,
    ) -> std::result::Result<(usize, Vec<Pair>), ReadlineError> {
        let Some(prefix) = completion_query(line, position) else {
            return Ok((position, Vec::new()));
        };

        let response = match self.runtime_handle.block_on(tokio::time::timeout(
            self.completion_timeout,
            get_slash_completions(
                &self.client,
                &self.base_url,
                &self.auth_token,
                &self.session_id,
                prefix,
            ),
        )) {
            Ok(Ok(response)) => response,
            Ok(Err(error)) => {
                eprintln!("[status] slash completion unavailable: {error}");
                return Ok((position, Vec::new()));
            }
            Err(_) => {
                eprintln!("[status] slash completion timed out");
                return Ok((position, Vec::new()));
            }
        };

        Ok((
            completion_start(prefix),
            response
                .candidates
                .iter()
                .map(|candidate| Pair {
                    display: completion_display(candidate),
                    replacement: candidate.insert_text.clone(),
                })
                .collect(),
        ))
    }
}

impl Highlighter for SlashCompletionHelper {}

impl Hinter for SlashCompletionHelper {
    type Hint = String;
}

impl Validator for SlashCompletionHelper {}

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

fn completion_query(line: &str, position: usize) -> Option<&str> {
    let prefix = &line[..position];
    classify_slash_completion_prefix(prefix).map(|_| prefix)
}

fn completion_start(prefix: &str) -> usize {
    prefix
        .rsplit_once(' ')
        .map_or(0, |(before, _)| before.len() + 1)
}

fn completion_display(candidate: &CompletionCandidate) -> String {
    let kind = match candidate.kind {
        CompletionKind::Command => "command",
        CompletionKind::Parameter => "parameter",
    };
    format!("{}\t{}\t{}", candidate.label, kind, candidate.detail)
}

#[cfg(test)]
mod tests;
