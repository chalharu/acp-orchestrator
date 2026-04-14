use super::support::*;

#[tokio::test]
async fn prompt_submission_streams_snapshot_user_and_assistant_messages() -> Result<()> {
    let stack = TestStack::spawn(ServerConfig {
        session_cap: 8,
        acp_server: String::new(),
    })
    .await?;

    let session = stack.create_session("alice").await?;
    let mut events = stack.open_events("alice", &session.session.id).await?;

    let snapshot = expect_next_event(&mut events).await?;
    assert!(matches!(
        snapshot.payload,
        StreamEventPayload::SessionSnapshot { .. }
    ));

    stack
        .submit_prompt("alice", &session.session.id, "hello through backend")
        .await?;

    let user_message = expect_next_event(&mut events).await?;
    match user_message.payload {
        StreamEventPayload::ConversationMessage { message } => {
            assert_eq!(message.text, "hello through backend");
            assert!(matches!(message.role, MessageRole::User));
        }
        payload => panic!("unexpected payload: {payload:?}"),
    }

    let assistant_message = expect_next_event(&mut events).await?;
    match assistant_message.payload {
        StreamEventPayload::ConversationMessage { message } => {
            assert!(matches!(message.role, MessageRole::Assistant));
            assert!(message.text.starts_with("mock assistant:"));
        }
        payload => panic!("unexpected payload: {payload:?}"),
    }

    Ok(())
}

#[tokio::test]
async fn session_lookup_rejects_different_principal() -> Result<()> {
    let stack = TestStack::spawn(ServerConfig {
        session_cap: 8,
        acp_server: String::new(),
    })
    .await?;
    let session = stack.create_session("alice").await?;

    let response = stack
        .client
        .get(format!(
            "{}/api/v1/sessions/{}",
            stack.backend_url, session.session.id
        ))
        .bearer_auth("bob")
        .send()
        .await
        .context("requesting session as the wrong principal")?;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    Ok(())
}

#[tokio::test]
async fn session_creation_enforces_principal_session_cap() -> Result<()> {
    let stack = TestStack::spawn(ServerConfig {
        session_cap: 1,
        acp_server: String::new(),
    })
    .await?;

    let first = stack.create_session("alice").await?;
    assert!(first.session.id.starts_with("s_"));

    let response = stack
        .client
        .post(format!("{}/api/v1/sessions", stack.backend_url))
        .bearer_auth("alice")
        .send()
        .await
        .context("creating a second session for alice")?;

    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    Ok(())
}

#[tokio::test]
async fn retention_prunes_oldest_closed_sessions() -> Result<()> {
    let stack = TestStack::spawn(ServerConfig {
        session_cap: 128,
        acp_server: String::new(),
    })
    .await?;

    let mut first_session_id = None;
    let mut last_session_id = None;

    for index in 0..33 {
        let created = stack.create_session("alice").await?;
        if index == 0 {
            first_session_id = Some(created.session.id.clone());
        }
        last_session_id = Some(created.session.id.clone());
        stack.close_session("alice", &created.session.id).await?;
    }

    let first_session_response = stack
        .client
        .get(format!(
            "{}/api/v1/sessions/{}",
            stack.backend_url,
            first_session_id.expect("first session id should exist")
        ))
        .bearer_auth("alice")
        .send()
        .await
        .context("loading the oldest closed session")?;
    assert_eq!(first_session_response.status(), StatusCode::NOT_FOUND);

    let last_session_response = stack
        .client
        .get(format!(
            "{}/api/v1/sessions/{}",
            stack.backend_url,
            last_session_id.expect("last session id should exist")
        ))
        .bearer_auth("alice")
        .send()
        .await
        .context("loading the newest closed session")?;
    assert_eq!(last_session_response.status(), StatusCode::OK);

    Ok(())
}

#[tokio::test]
async fn session_history_returns_messages_after_a_roundtrip() -> Result<()> {
    let stack = TestStack::spawn(ServerConfig {
        session_cap: 8,
        acp_server: String::new(),
    })
    .await?;
    let session = stack.create_session("alice").await?;
    let mut events = stack.open_events("alice", &session.session.id).await?;

    let snapshot = expect_next_event(&mut events).await?;
    assert!(matches!(
        snapshot.payload,
        StreamEventPayload::SessionSnapshot { .. }
    ));

    stack
        .submit_prompt("alice", &session.session.id, "history please")
        .await?;
    let _ = expect_next_event(&mut events).await?;
    let _ = expect_next_event(&mut events).await?;

    let history = stack.session_history("alice", &session.session.id).await?;
    assert_eq!(history.messages.len(), 2);
    assert!(matches!(history.messages[0].role, MessageRole::User));
    assert_eq!(history.messages[0].text, "history please");
    assert!(matches!(history.messages[1].role, MessageRole::Assistant));

    Ok(())
}

#[tokio::test]
async fn prompt_submission_streams_mock_failures_as_status_messages() -> Result<()> {
    let stack = TestStack::spawn(ServerConfig {
        session_cap: 8,
        acp_server: "127.0.0.1:9".to_string(),
    })
    .await?;
    let session = stack.create_session("alice").await?;
    let mut events = stack.open_events("alice", &session.session.id).await?;

    let snapshot = expect_next_event(&mut events).await?;
    assert!(matches!(
        snapshot.payload,
        StreamEventPayload::SessionSnapshot { .. }
    ));

    stack
        .submit_prompt("alice", &session.session.id, "this will fail")
        .await?;

    let user_message = expect_next_event(&mut events).await?;
    assert!(matches!(
        user_message.payload,
        StreamEventPayload::ConversationMessage { message }
            if matches!(message.role, MessageRole::User)
    ));

    let status = expect_next_event(&mut events).await?;
    assert!(matches!(
        status.payload,
        StreamEventPayload::Status { message } if message.starts_with("ACP request failed:")
    ));

    Ok(())
}
