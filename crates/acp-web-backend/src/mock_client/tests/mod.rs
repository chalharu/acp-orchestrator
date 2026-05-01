use super::*;
use crate::sessions::{PendingPrompt, SessionStore};
use crate::support::http::wait_for_tcp_connect;
use acp_mock::{MockConfig, spawn_with_shutdown_task};
use tokio::{net::TcpListener, sync::oneshot};

mod backend_client;
mod helpers;
mod request_reply;

async fn spawn_mock_server(delay: Duration) -> (String, oneshot::Sender<()>) {
    spawn_mock_server_with_config(MockConfig {
        response_delay: delay,
        ..MockConfig::default()
    })
    .await
}

async fn spawn_mock_server_with_config(config: MockConfig) -> (String, oneshot::Sender<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let address = listener
        .local_addr()
        .expect("listener should expose its address");
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    spawn_with_shutdown_task(listener, config, async move {
        let _ = shutdown_rx.await;
    });

    wait_for_tcp_connect(&address.to_string(), 20, Duration::from_millis(10))
        .await
        .expect("mock server should accept TCP connections");

    (address.to_string(), shutdown_tx)
}

async fn test_pending_prompt(owner: &str, prompt: &str) -> PendingPrompt {
    let store = SessionStore::new(4);
    let session = store
        .create_session(owner, "w_test")
        .await
        .expect("session creation should succeed");
    store
        .submit_prompt(owner, &session.id, prompt.to_string())
        .await
        .expect("prompt submission should succeed")
}

#[tokio::test]
async fn mock_client_uses_session_acp_address_override() {
    let client = MockClient::with_timeout("127.0.0.1:1".to_string(), Duration::from_secs(1))
        .expect("client should build");
    client
        .bind_session_launch_metadata(
            "s_test",
            crate::agent_runtime::AgentLaunchMetadata {
                acp_address: Some("127.0.0.1:2".to_string()),
            },
        )
        .await
        .expect("metadata bind should succeed");

    assert_eq!(client.session_acp_address("s_test").await, "127.0.0.1:2");
    assert_eq!(client.session_acp_address("s_other").await, "127.0.0.1:1");

    client
        .bind_session_launch_metadata(
            "s_empty",
            crate::agent_runtime::AgentLaunchMetadata::default(),
        )
        .await
        .expect("empty metadata should bind without changing addresses");
    assert_eq!(client.session_acp_address("s_empty").await, "127.0.0.1:1");
}

#[test]
fn create_session_error_includes_acp_source_details() {
    let error = MockClientError::CreateSession {
        source: acp::Error::invalid_params().data("cwd must be absolute"),
    };
    let message = error.to_string();

    assert!(message.contains("creating an ACP session failed"));
    assert!(message.contains("Invalid params"));
    assert!(message.contains("cwd must be absolute"));
}
