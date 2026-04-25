use super::*;
use crate::contract_messages::{ConversationMessage, MessageRole};
use crate::contract_sessions::SessionStatus;
use crate::contract_stream::StreamEventPayload;
use chrono::{TimeZone, Utc};

mod history;
mod permissions;

fn list_item(
    id: &str,
    workspace_id: &str,
    last_activity_at: chrono::DateTime<Utc>,
) -> SessionListItem {
    SessionListItem {
        id: id.to_string(),
        workspace_id: workspace_id.to_string(),
        title: "New chat".to_string(),
        status: SessionStatus::Active,
        last_activity_at,
    }
}

#[tokio::test]
async fn delete_sessions_for_owners_returns_empty_for_empty_owner_lists() {
    let store = SessionStore::new(4);

    assert!(store.delete_sessions_for_owners(&[]).await.is_empty());
}

#[tokio::test]
async fn delete_sessions_for_owners_removes_matching_sessions() {
    let store = SessionStore::new(4);
    let alice = store
        .create_session("alice", "w_test")
        .await
        .expect("alice session creation should succeed");
    let bob = store
        .create_session("bob", "w_test")
        .await
        .expect("bob session creation should succeed");

    let removed = store
        .delete_sessions_for_owners(&["alice".to_string()])
        .await;

    assert_eq!(removed, vec![alice.id.clone()]);
    assert_eq!(
        store.session_snapshot("alice", &alice.id).await,
        Err(SessionStoreError::NotFound)
    );
    assert!(store.session_snapshot("bob", &bob.id).await.is_ok());
}

#[test]
fn compare_session_entries_prefers_newer_activity_when_recent_order_matches() {
    let older = Utc.timestamp_opt(1_700_000_000, 0).single().unwrap();
    let newer = Utc.timestamp_opt(1_700_000_100, 0).single().unwrap();

    let left = (7, list_item("s_old", "w_test", older));
    let right = (7, list_item("s_new", "w_test", newer));

    assert_eq!(
        compare_session_entries(&left, &right),
        std::cmp::Ordering::Greater
    );
    assert_eq!(
        compare_session_entries(&right, &left),
        std::cmp::Ordering::Less
    );
}

#[test]
fn compare_session_entries_falls_back_to_ids_when_other_fields_match() {
    let timestamp = Utc.timestamp_opt(1_700_000_000, 0).single().unwrap();
    let left = (7, list_item("s_a", "w_test", timestamp));
    let right = (7, list_item("s_b", "w_test", timestamp));

    assert_eq!(
        compare_session_entries(&left, &right),
        std::cmp::Ordering::Less
    );
    assert_eq!(
        compare_session_entries(&right, &left),
        std::cmp::Ordering::Greater
    );
}

#[tokio::test]
async fn list_workspace_sessions_filters_by_owner_and_workspace() {
    let store = SessionStore::new(4);
    let first = store
        .create_session("alice", "w_alpha")
        .await
        .expect("first session creation should succeed");
    let second = store
        .create_session("alice", "w_alpha")
        .await
        .expect("second session creation should succeed");
    let _other_workspace = store
        .create_session("alice", "w_beta")
        .await
        .expect("other workspace session creation should succeed");
    let _other_owner = store
        .create_session("bob", "w_alpha")
        .await
        .expect("other owner session creation should succeed");

    let listed = store.list_workspace_sessions("alice", "w_alpha").await;

    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].id, second.id);
    assert_eq!(listed[1].id, first.id);
    assert!(
        listed
            .iter()
            .all(|session| session.workspace_id == "w_alpha")
    );
}

#[tokio::test]
async fn restore_session_rejects_foreign_owners_when_the_live_session_already_exists() {
    let store = SessionStore::new(4);
    let existing = store
        .create_session("alice", "w_test")
        .await
        .expect("live session creation should succeed");

    let error = store
        .restore_session(
            "bob",
            SessionSnapshot {
                id: existing.id,
                workspace_id: "w_test".to_string(),
                title: "Restored".to_string(),
                status: SessionStatus::Active,
                latest_sequence: 1,
                messages: vec![ConversationMessage {
                    id: "m_user".to_string(),
                    role: MessageRole::User,
                    text: "hello".to_string(),
                    created_at: Utc::now(),
                }],
                pending_permissions: Vec::new(),
            },
            Utc::now(),
        )
        .await
        .expect_err("restoring a foreign-owned live session should fail");

    assert_eq!(error, SessionStoreError::Forbidden);
}
