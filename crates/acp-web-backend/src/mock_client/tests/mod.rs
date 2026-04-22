use super::*;
use crate::sessions::{PendingPrompt, SessionStore};
use acp_app_support::wait_for_tcp_connect;
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
        .create_session(owner)
        .await
        .expect("session creation should succeed");
    store
        .submit_prompt(owner, &session.id, prompt.to_string())
        .await
        .expect("prompt submission should succeed")
}
