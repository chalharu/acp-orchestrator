use super::*;

#[tokio::test]
async fn apply_command_outcome_handles_refresh_failures() {
    let client = Client::builder().build().expect("client should build");
    let runtime_handle = Handle::current();
    let server_url = "http://127.0.0.1:9".to_string();
    let auth_token = "developer".to_string();
    let session_id = "s_test".to_string();

    let app = tokio::task::spawn_blocking(move || {
        let context = build_context(
            &runtime_handle,
            &client,
            &server_url,
            &auth_token,
            &session_id,
        );
        let mut app = ChatApp::new("s_test", "http://127.0.0.1:8080", false, &[], &[], vec![]);
        apply_command_outcome(
            &context,
            &mut app,
            crate::repl_commands::ReplCommandOutcome {
                notices: vec![],
                pending_permissions_update: crate::repl_commands::PendingPermissionsUpdate::Refresh,
                should_quit: false,
            },
        )
        .expect("refresh failures should stay in-process");
        app
    })
    .await
    .expect("refresh worker should join");

    assert!(
        app.status_entries()
            .iter()
            .any(|status| status == "load session request failed")
    );
}

#[tokio::test]
async fn apply_command_outcome_removes_pending_permissions() {
    let client = Client::builder().build().expect("client should build");
    let runtime_handle = Handle::current();
    let server_url = "http://127.0.0.1:9".to_string();
    let auth_token = "developer".to_string();
    let session_id = "s_test".to_string();

    let app = tokio::task::spawn_blocking(move || {
        let context = build_context(
            &runtime_handle,
            &client,
            &server_url,
            &auth_token,
            &session_id,
        );
        let mut app = ChatApp::new(
            "s_test",
            "http://127.0.0.1:8080",
            false,
            &[],
            &[PermissionRequest {
                request_id: "req_1".to_string(),
                summary: "old".to_string(),
            }],
            vec![],
        );
        apply_command_outcome(
            &context,
            &mut app,
            crate::repl_commands::ReplCommandOutcome {
                notices: vec![],
                pending_permissions_update: crate::repl_commands::PendingPermissionsUpdate::Remove(
                    "req_1".to_string(),
                ),
                should_quit: false,
            },
        )
        .expect("removals should stay in-process");
        app
    })
    .await
    .expect("remove worker should join");

    assert!(app.pending_permissions().is_empty());
    assert!(!app.should_quit());
}
