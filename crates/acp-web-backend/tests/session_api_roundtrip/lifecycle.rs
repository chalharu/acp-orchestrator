use super::support::*;
use std::time::Duration;

#[tokio::test]
async fn lagged_event_streams_continue_after_dropping_backlog() -> Result<()> {
    let stack = TestStack::spawn(ServerConfig {
        session_cap: 8,
        acp_server: "127.0.0.1:9".to_string(),
        startup_hints: false,
    })
    .await?;
    let session = stack.create_session("alice").await?;
    let mut events = stack.open_events("alice", &session.session.id).await?;

    for index in 0..80 {
        stack
            .submit_prompt("alice", &session.session.id, &format!("prompt {index}"))
            .await?;
    }
    sleep(Duration::from_millis(200)).await;

    let snapshot = expect_next_event(&mut events).await?;
    assert!(matches!(
        snapshot.payload,
        StreamEventPayload::SessionSnapshot { .. }
    ));

    let resumed = expect_next_event(&mut events).await?;
    assert!(matches!(
        resumed.payload,
        StreamEventPayload::ConversationMessage { .. } | StreamEventPayload::Status { .. }
    ));

    Ok(())
}

#[tokio::test]
async fn direct_mock_server_accepts_tcp_connections() -> Result<()> {
    let (address, shutdown, handle) = spawn_direct_mock_server().await?;

    acp_app_support::wait_for_tcp_connect(&address, 50, Duration::from_millis(50))
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;

    let _ = shutdown.send(());
    handle
        .await
        .context("joining direct mock task")?
        .context("mock server should stop cleanly")?;
    Ok(())
}

#[tokio::test]
async fn direct_backend_server_reports_health() -> Result<()> {
    let (base_url, handle) = spawn_direct_backend_server("127.0.0.1:9".to_string()).await?;
    let client = acp_app_support::build_http_client_for_url(&base_url, None)
        .context("building test client")?;

    acp_app_support::wait_for_health(&client, &base_url, 50, Duration::from_millis(50))
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;

    handle.abort();
    let _ = handle.await;
    Ok(())
}

#[tokio::test]
async fn graceful_mock_server_shutdown_completes_cleanly() -> Result<()> {
    let (address, shutdown, handle) = spawn_graceful_mock_server().await?;

    acp_app_support::wait_for_tcp_connect(&address, 50, Duration::from_millis(50))
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    let _ = shutdown.send(());
    handle
        .await
        .context("joining graceful mock task")?
        .context("mock server should stop cleanly")?;

    Ok(())
}

#[tokio::test]
async fn graceful_backend_server_shutdown_completes_cleanly() -> Result<()> {
    let (base_url, shutdown, handle) =
        spawn_graceful_backend_server("127.0.0.1:9".to_string()).await?;
    let client = acp_app_support::build_http_client_for_url(&base_url, None)
        .context("building test client")?;

    acp_app_support::wait_for_health(&client, &base_url, 50, Duration::from_millis(50))
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    let _ = shutdown.send(());
    handle
        .await
        .context("joining graceful backend task")?
        .context("backend server should stop cleanly")?;

    Ok(())
}
