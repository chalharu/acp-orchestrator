use super::*;
use crate::contract_messages::MessageRole;
use crate::contract_stream::StreamEventPayload;

mod history;
mod permissions;

#[tokio::test]
async fn delete_sessions_for_owners_returns_empty_for_empty_owner_lists() {
    let store = SessionStore::new(4);

    assert!(store.delete_sessions_for_owners(&[]).await.is_empty());
}

#[tokio::test]
async fn delete_sessions_for_owners_removes_matching_sessions() {
    let store = SessionStore::new(4);
    let alice = store
        .create_session("alice")
        .await
        .expect("alice session creation should succeed");
    let bob = store
        .create_session("bob")
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
