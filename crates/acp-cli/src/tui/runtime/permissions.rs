use tokio::sync::mpsc;

use super::super::{TuiEvent, app::ChatApp, input::UiContext};
use crate::contract_messages::MessageRole;

#[derive(Default)]
pub(super) struct PendingPermissionRefreshState {
    pub(super) in_flight: bool,
    pub(super) queued: bool,
}

pub(super) fn drain_events(
    event_rx: &mut mpsc::UnboundedReceiver<TuiEvent>,
    app: &mut ChatApp,
    refresh_state: &mut PendingPermissionRefreshState,
) {
    let mut queue_pending_permission_refresh = false;
    loop {
        match event_rx.try_recv() {
            Ok(TuiEvent::Stream(update)) => {
                queue_pending_permission_refresh |=
                    should_refresh_pending_permissions(app, &update);
                app.apply_stream_update(update);
            }
            Ok(TuiEvent::StreamEnded(message)) => app.set_connection_lost(message),
            Ok(TuiEvent::PendingPermissionsRefreshed(result)) => {
                refresh_state.in_flight = false;
                match result {
                    Ok(pending_permissions) => app.replace_pending_permissions(pending_permissions),
                    Err(error) => app.push_status(error),
                }
            }
            Err(mpsc::error::TryRecvError::Empty)
            | Err(mpsc::error::TryRecvError::Disconnected) => {
                if queue_pending_permission_refresh {
                    refresh_state.queued = true;
                }
                return;
            }
        }
    }
}

fn should_refresh_pending_permissions(app: &ChatApp, update: &crate::events::StreamUpdate) -> bool {
    if app.pending_permissions().is_empty() {
        return false;
    }

    match update {
        crate::events::StreamUpdate::Status(_)
        | crate::events::StreamUpdate::SessionClosed { .. } => true,
        crate::events::StreamUpdate::ConversationMessage(message) => {
            matches!(message.role, MessageRole::Assistant)
        }
        crate::events::StreamUpdate::PermissionRequested(_) => false,
    }
}

pub(super) fn launch_pending_permission_refresh(
    context: &UiContext<'_>,
    event_tx: &mpsc::UnboundedSender<TuiEvent>,
    refresh_state: &mut PendingPermissionRefreshState,
) {
    if refresh_state.in_flight || !refresh_state.queued {
        return;
    }

    refresh_state.in_flight = true;
    refresh_state.queued = false;

    let client = context.client.clone();
    let server_url = context.server_url.to_string();
    let auth_token = context.auth_token.to_string();
    let session_id = context.session_id.to_string();
    let event_tx = event_tx.clone();
    context.runtime_handle.spawn(async move {
        let result = crate::api::get_session(&client, &server_url, &auth_token, &session_id)
            .await
            .map(|session| session.pending_permissions)
            .map_err(|error| error.to_string());
        let _ = event_tx.send(TuiEvent::PendingPermissionsRefreshed(result));
    });
}
