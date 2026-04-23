use crate::contract_slash::{
    CompletionCandidate, CompletionKind, SLASH_COMMAND_SPECS, SlashCompletionQuery,
    SlashCompletionsResponse, classify_slash_completion_prefix,
};

use crate::sessions::{SessionStore, SessionStoreError};

pub(crate) async fn resolve_slash_completions(
    store: &SessionStore,
    owner: &str,
    session_id: &str,
    prefix: &str,
) -> Result<SlashCompletionsResponse, SessionStoreError> {
    let candidates = match classify_slash_completion_prefix(prefix) {
        Some(SlashCompletionQuery::RequestId {
            prefix: request_prefix,
            ..
        }) => {
            let pending_permissions = store.session_pending_permissions(owner, session_id).await?;
            pending_permissions
                .iter()
                .filter(|request| request.request_id.starts_with(request_prefix))
                .map(|request| CompletionCandidate {
                    label: request.request_id.clone(),
                    insert_text: request.request_id.clone(),
                    detail: request.summary.clone(),
                    kind: CompletionKind::Parameter,
                })
                .collect()
        }
        Some(SlashCompletionQuery::Commands {
            prefix: command_prefix,
        }) => {
            store.ensure_session_access(owner, session_id).await?;
            command_candidates(command_prefix)
        }
        None => {
            store.ensure_session_access(owner, session_id).await?;
            Vec::new()
        }
    };

    Ok(SlashCompletionsResponse { candidates })
}

fn command_candidates(prefix: &str) -> Vec<CompletionCandidate> {
    SLASH_COMMAND_SPECS
        .iter()
        .filter(|command| command.name.starts_with(prefix))
        .map(|command| CompletionCandidate {
            label: command.label.to_string(),
            insert_text: command.insert_text.to_string(),
            detail: command.detail.to_string(),
            kind: CompletionKind::Command,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn command_completion_filters_the_static_catalog_by_prefix() {
        let store = SessionStore::new(4);
        let session = store
            .create_session("alice", "w_test")
            .await
            .expect("session creation should succeed");

        let response = resolve_slash_completions(&store, "alice", &session.id, "/a")
            .await
            .expect("command completion should succeed");

        assert_eq!(
            response.candidates,
            vec![CompletionCandidate {
                label: "/approve <request-id>".to_string(),
                insert_text: "/approve ".to_string(),
                detail: "Approve a pending permission request".to_string(),
                kind: CompletionKind::Command,
            }]
        );
    }

    #[tokio::test]
    async fn permission_argument_completion_uses_pending_request_ids() {
        let store = SessionStore::new(4);
        let session = store
            .create_session("alice", "w_test")
            .await
            .expect("session creation should succeed");
        let pending = store
            .submit_prompt("alice", &session.id, "verify permission".to_string())
            .await
            .expect("prompt submission should succeed");
        let turn = pending.turn_handle();
        let _cancel_rx = turn
            .start_turn()
            .await
            .expect("starting the turn should succeed");
        let _resolution = turn
            .register_permission_request(
                "read_text_file README.md".to_string(),
                "allow_once".to_string(),
                "reject_once".to_string(),
            )
            .await
            .expect("permission request registration should succeed");

        let response = resolve_slash_completions(&store, "alice", &session.id, "/approve req_")
            .await
            .expect("parameter completion should succeed");

        assert_eq!(
            response.candidates,
            vec![CompletionCandidate {
                label: "req_1".to_string(),
                insert_text: "req_1".to_string(),
                detail: "read_text_file README.md".to_string(),
                kind: CompletionKind::Parameter,
            }]
        );
    }

    #[tokio::test]
    async fn non_slash_prefixes_return_no_candidates() {
        let store = SessionStore::new(4);
        let session = store
            .create_session("alice", "w_test")
            .await
            .expect("session creation should succeed");

        let response = resolve_slash_completions(&store, "alice", &session.id, "hello")
            .await
            .expect("non-slash prefixes should be accepted");

        assert!(response.candidates.is_empty());
    }

    #[tokio::test]
    async fn unsupported_slash_prefixes_return_no_candidates() {
        let store = SessionStore::new(4);
        let session = store
            .create_session("alice", "w_test")
            .await
            .expect("session creation should succeed");

        let response = resolve_slash_completions(&store, "alice", &session.id, "/home/alice")
            .await
            .expect("unsupported slash prefixes should be ignored");

        assert!(response.candidates.is_empty());
    }

    #[tokio::test]
    async fn owner_checks_apply_to_completion_queries() {
        let store = SessionStore::new(4);
        let session = store
            .create_session("alice", "w_test")
            .await
            .expect("session creation should succeed");

        let error = resolve_slash_completions(&store, "bob", &session.id, "/")
            .await
            .expect_err("other owners must not see slash completions");

        assert_eq!(error, SessionStoreError::Forbidden);
    }
}
