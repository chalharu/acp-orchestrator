use super::support::*;

async fn assert_invalid_rename_title(title: String, expected_message: &str) -> Result<()> {
    let stack = TestStack::spawn(ServerConfig {
        session_cap: 8,
        acp_server: String::new(),
        startup_hints: false,
        state_dir: test_state_dir(),
        frontend_dist: None,
    })
    .await?;
    let session = stack.create_session("alice").await?;

    let response = stack
        .client
        .patch(format!(
            "{}/api/v1/sessions/{}",
            stack.backend_url, session.session.id
        ))
        .bearer_auth("alice")
        .json(&acp_contracts::RenameSessionRequest { title })
        .send()
        .await
        .context("sending invalid rename request")?;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(
        response
            .text()
            .await
            .context("reading invalid rename response")?
            .contains(expected_message)
    );

    Ok(())
}

#[tokio::test]
async fn prompt_submission_streams_snapshot_user_and_assistant_messages() -> Result<()> {
    let stack = TestStack::spawn(ServerConfig {
        session_cap: 8,
        acp_server: String::new(),
        startup_hints: false,
        state_dir: test_state_dir(),
        frontend_dist: None,
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
        startup_hints: false,
        state_dir: test_state_dir(),
        frontend_dist: None,
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
async fn rename_and_delete_reject_different_principal() -> Result<()> {
    let stack = TestStack::spawn(ServerConfig {
        session_cap: 8,
        acp_server: String::new(),
        startup_hints: false,
        state_dir: test_state_dir(),
        frontend_dist: None,
    })
    .await?;
    let session = stack.create_session("alice").await?;

    let rename_response = stack
        .client
        .patch(format!(
            "{}/api/v1/sessions/{}",
            stack.backend_url, session.session.id
        ))
        .bearer_auth("bob")
        .json(&acp_contracts::RenameSessionRequest {
            title: "hijack".to_string(),
        })
        .send()
        .await
        .context("renaming session as the wrong principal")?;
    assert_eq!(rename_response.status(), StatusCode::FORBIDDEN);

    let delete_response = stack
        .client
        .delete(format!(
            "{}/api/v1/sessions/{}",
            stack.backend_url, session.session.id
        ))
        .bearer_auth("bob")
        .send()
        .await
        .context("deleting session as the wrong principal")?;
    assert_eq!(delete_response.status(), StatusCode::FORBIDDEN);

    Ok(())
}

#[tokio::test]
async fn session_creation_enforces_principal_session_cap() -> Result<()> {
    let stack = TestStack::spawn(ServerConfig {
        session_cap: 1,
        acp_server: String::new(),
        startup_hints: false,
        state_dir: test_state_dir(),
        frontend_dist: None,
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
async fn session_list_is_owner_scoped_and_keeps_retained_closed_sessions() -> Result<()> {
    let stack = TestStack::spawn(ServerConfig {
        session_cap: 8,
        acp_server: String::new(),
        startup_hints: false,
        state_dir: test_state_dir(),
        frontend_dist: None,
    })
    .await?;

    let first = stack.create_session("alice").await?;
    let second = stack.create_session("alice").await?;
    let bob = stack.create_session("bob").await?;
    stack.close_session("alice", &first.session.id).await?;

    let sessions = stack.list_sessions("alice").await?;

    // Closing no longer reorders: second (created more recently) stays at index 0.
    assert_eq!(sessions.sessions.len(), 2);
    assert_eq!(sessions.sessions[0].id, second.session.id);
    assert_eq!(
        sessions.sessions[0].status,
        acp_contracts::SessionStatus::Active
    );
    assert_eq!(sessions.sessions[1].id, first.session.id);
    assert_eq!(
        sessions.sessions[1].status,
        acp_contracts::SessionStatus::Closed
    );
    assert!(
        sessions
            .sessions
            .iter()
            .all(|session| session.id != bob.session.id)
    );
    Ok(())
}

#[tokio::test]
async fn getting_a_session_does_not_reorder_the_owned_session_list() -> Result<()> {
    let stack = TestStack::spawn(ServerConfig {
        session_cap: 8,
        acp_server: String::new(),
        startup_hints: false,
        state_dir: test_state_dir(),
        frontend_dist: None,
    })
    .await?;

    let first = stack.create_session("alice").await?;
    let second = stack.create_session("alice").await?;

    let before = stack.list_sessions("alice").await?;
    assert_eq!(
        before
            .sessions
            .iter()
            .map(|session| session.id.as_str())
            .collect::<Vec<_>>(),
        vec![second.session.id.as_str(), first.session.id.as_str()]
    );

    // GET should not change the ordering.
    let _ = stack.session_snapshot("alice", &first.session.id).await?;

    let after = stack.list_sessions("alice").await?;
    assert_eq!(
        after
            .sessions
            .iter()
            .map(|session| session.id.as_str())
            .collect::<Vec<_>>(),
        vec![second.session.id.as_str(), first.session.id.as_str()],
        "GET must not reorder the session list"
    );
    Ok(())
}

#[tokio::test]
async fn prompt_submission_moves_session_to_front_of_list() -> Result<()> {
    let stack = TestStack::spawn(ServerConfig {
        session_cap: 8,
        acp_server: "127.0.0.1:9".to_string(),
        startup_hints: false,
        state_dir: test_state_dir(),
        frontend_dist: None,
    })
    .await?;

    let first = stack.create_session("alice").await?;
    let second = stack.create_session("alice").await?;

    let before = stack.list_sessions("alice").await?;
    assert_eq!(before.sessions[0].id, second.session.id);
    assert_eq!(before.sessions[1].id, first.session.id);

    // Submitting a prompt bumps the session to the top of the list.
    stack
        .submit_prompt("alice", &first.session.id, "hello")
        .await?;

    let after = stack.list_sessions("alice").await?;
    assert_eq!(after.sessions[0].id, first.session.id);
    assert_eq!(after.sessions[1].id, second.session.id);
    Ok(())
}

#[tokio::test]
async fn session_title_defaults_to_new_chat_and_auto_sets_from_first_prompt() -> Result<()> {
    let stack = TestStack::spawn(ServerConfig {
        session_cap: 8,
        acp_server: "127.0.0.1:9".to_string(),
        startup_hints: false,
        state_dir: test_state_dir(),
        frontend_dist: None,
    })
    .await?;

    let created = stack.create_session("alice").await?;
    assert_eq!(
        created.session.title, "New chat",
        "freshly created session should have the default title"
    );

    stack
        .submit_prompt("alice", &created.session.id, "What is 2+2?")
        .await?;

    let after = stack.session_snapshot("alice", &created.session.id).await?;
    assert_eq!(
        after.session.title, "What is 2+2?",
        "title should auto-set from the first user prompt"
    );

    // Submitting a second prompt must not overwrite the auto-set title.
    stack
        .submit_prompt("alice", &created.session.id, "Follow-up question")
        .await?;
    let after2 = stack.session_snapshot("alice", &created.session.id).await?;
    assert_eq!(after2.session.title, "What is 2+2?");

    Ok(())
}

#[tokio::test]
async fn session_can_be_renamed_and_title_appears_in_list_and_snapshot() -> Result<()> {
    let stack = TestStack::spawn(ServerConfig {
        session_cap: 8,
        acp_server: String::new(),
        startup_hints: false,
        state_dir: test_state_dir(),
        frontend_dist: None,
    })
    .await?;

    let created = stack.create_session("alice").await?;
    stack
        .rename_session("alice", &created.session.id, "My renamed session")
        .await?;

    let snapshot = stack.session_snapshot("alice", &created.session.id).await?;
    assert_eq!(snapshot.session.title, "My renamed session");

    let list = stack.list_sessions("alice").await?;
    assert_eq!(list.sessions[0].title, "My renamed session");

    Ok(())
}

#[tokio::test]
async fn rename_session_rejects_blank_titles() -> Result<()> {
    assert_invalid_rename_title("   ".to_string(), "title must not be empty").await
}

#[tokio::test]
async fn rename_session_rejects_titles_over_500_characters() -> Result<()> {
    assert_invalid_rename_title("x".repeat(501), "title must not exceed 500 characters").await
}

#[tokio::test]
async fn manual_rename_prevents_auto_title_from_first_prompt() -> Result<()> {
    let stack = TestStack::spawn(ServerConfig {
        session_cap: 8,
        acp_server: "127.0.0.1:9".to_string(),
        startup_hints: false,
        state_dir: test_state_dir(),
        frontend_dist: None,
    })
    .await?;

    let created = stack.create_session("alice").await?;
    stack
        .rename_session("alice", &created.session.id, "My custom title")
        .await?;

    stack
        .submit_prompt("alice", &created.session.id, "First message")
        .await?;

    let after = stack.session_snapshot("alice", &created.session.id).await?;
    assert_eq!(
        after.session.title, "My custom title",
        "manual rename must survive subsequent prompt submission"
    );

    Ok(())
}

#[tokio::test]
async fn session_can_be_deleted_and_is_no_longer_accessible() -> Result<()> {
    let stack = TestStack::spawn(ServerConfig {
        session_cap: 8,
        acp_server: String::new(),
        startup_hints: false,
        state_dir: test_state_dir(),
        frontend_dist: None,
    })
    .await?;

    let created = stack.create_session("alice").await?;
    let session_id = created.session.id.clone();

    stack.delete_session("alice", &session_id).await?;

    let list = stack.list_sessions("alice").await?;
    assert!(
        list.sessions.is_empty(),
        "deleted session must not appear in the list"
    );

    let response = stack
        .client
        .get(format!(
            "{}/api/v1/sessions/{session_id}",
            stack.backend_url
        ))
        .bearer_auth("alice")
        .send()
        .await
        .context("requesting deleted session")?;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    Ok(())
}

#[tokio::test]
async fn retention_prunes_oldest_closed_sessions() -> Result<()> {
    let stack = TestStack::spawn(ServerConfig {
        session_cap: 128,
        acp_server: String::new(),
        startup_hints: false,
        state_dir: test_state_dir(),
        frontend_dist: None,
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
        startup_hints: false,
        state_dir: test_state_dir(),
        frontend_dist: None,
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
        startup_hints: false,
        state_dir: test_state_dir(),
        frontend_dist: None,
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
