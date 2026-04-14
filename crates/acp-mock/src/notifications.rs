use crate::{
    PermissionRequester, QueuedPermissionRequest, QueuedSessionNotification, SessionUpdateNotifier,
};
use agent_client_protocol as acp;
use tokio::sync::{mpsc, oneshot};
use tracing::error;

pub(super) fn log_connection_result(result: Result<(), acp::Error>) {
    if let Err(error) = result {
        error!("mock ACP connection failed: {error}");
    }
}

pub(super) fn finalize_session_update(
    result: Result<(), acp::Error>,
    ack_tx: oneshot::Sender<()>,
) -> bool {
    if let Err(error) = result {
        error!("sending mock ACP session update failed: {error}");
        return false;
    }

    let _ = ack_tx.send(());
    true
}

pub(super) async fn drain_session_updates<N: SessionUpdateNotifier>(
    notifier: &N,
    mut session_update_rx: mpsc::UnboundedReceiver<QueuedSessionNotification>,
) {
    while let Some((notification, ack_tx)) = session_update_rx.recv().await {
        let result = notifier.send_session_update(notification).await;
        if !finalize_session_update(result, ack_tx) {
            return;
        }
    }
}

pub(super) fn finalize_permission_request(
    result: Result<acp::RequestPermissionResponse, acp::Error>,
    ack_tx: oneshot::Sender<Result<acp::RequestPermissionResponse, acp::Error>>,
) -> bool {
    let should_continue = result.is_ok();
    if let Err(error) = ack_tx.send(result) {
        error!("sending mock ACP permission outcome failed: {error:?}");
        return false;
    }
    should_continue
}

pub(super) async fn drain_permission_requests<N: PermissionRequester>(
    requester: &N,
    mut permission_request_rx: mpsc::UnboundedReceiver<QueuedPermissionRequest>,
) {
    while let Some((request, ack_tx)) = permission_request_rx.recv().await {
        let result = requester.request_permission(request).await;
        if !finalize_permission_request(result, ack_tx) {
            break;
        }
    }
}
