use std::{
    io,
    path::PathBuf,
    process::Stdio,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use tokio::{io::AsyncWriteExt, process::Command, time::sleep};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[tokio::test]
async fn launcher_starts_the_full_stack_and_proxies_cli_io() -> Result<()> {
    let recent_path = unique_recent_sessions_path("launcher");
    let mut child = Command::new(env!("CARGO_BIN_EXE_acp"))
        .env("ACP_RECENT_SESSIONS_PATH", &recent_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("missing launcher stdin"))?;

    stdin.write_all(b"hello from launcher\n").await?;
    sleep(Duration::from_millis(300)).await;
    stdin.write_all(b"/quit\n").await?;
    drop(stdin);

    let output = child.wait_with_output().await?;
    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("session: s_"));
    assert!(stdout.contains("connected to backend: http://127.0.0.1:"));
    assert!(stdout.contains("[assistant]"));

    Ok(())
}

fn unique_recent_sessions_path(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after the epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("acp-launcher-{label}-{nanos}.json"))
}
