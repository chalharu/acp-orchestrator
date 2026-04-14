use std::collections::HashSet;

use super::*;
use acp_contracts::PermissionRequest;
use eventsource_stream::Eventsource;
use futures_util::{StreamExt, pin_mut};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct InitialSnapshotState {
    message_ids: HashSet<String>,
    permission_request_ids: HashSet<String>,
}

impl InitialSnapshotState {
    pub(super) fn from_snapshot(snapshot: &SessionSnapshot) -> Self {
        Self {
            message_ids: snapshot
                .messages
                .iter()
                .map(|message| message.id.clone())
                .collect(),
            permission_request_ids: snapshot
                .pending_permissions
                .iter()
                .map(|request| request.request_id.clone())
                .collect(),
        }
    }
}

pub(super) async fn stream_events_to_stderr(
    client: Client,
    events_url: String,
    auth_token: String,
    initial_snapshot_state: Option<InitialSnapshotState>,
) {
    if let Err(error) = stream_events(client, events_url, auth_token, initial_snapshot_state).await
    {
        eprintln!("[status] event stream ended: {error}");
    }
}

pub(super) async fn stream_events(
    client: Client,
    events_url: String,
    auth_token: String,
    initial_snapshot_state: Option<InitialSnapshotState>,
) -> Result<()> {
    let response = client
        .get(events_url)
        .bearer_auth(auth_token)
        .send()
        .await
        .context(SendRequestSnafu {
            action: "open event stream",
        })?;
    let response = ensure_success(response, "open event stream").await?;
    let stream = response.bytes_stream().eventsource();
    pin_mut!(stream);
    let mut initial_snapshot_state = initial_snapshot_state;

    while let Some(event) = stream.next().await {
        let event = event.map_err(|source| CliError::ReadEventStream {
            source: Box::new(source),
        })?;
        let payload: StreamEvent =
            serde_json::from_str(&event.data).context(DecodeStreamEventSnafu)?;
        if let StreamEventPayload::SessionSnapshot { session } = &payload.payload
            && let Some(known_snapshot_state) = initial_snapshot_state.take()
        {
            render_initial_snapshot_delta(session, &known_snapshot_state);
            continue;
        }
        render_event(&payload);
    }

    Ok(())
}

pub(super) fn render_event(event: &StreamEvent) {
    match &event.payload {
        StreamEventPayload::SessionSnapshot { session } => {
            if session.messages.is_empty() && session.pending_permissions.is_empty() {
                println!("[status] session ready");
            } else {
                for message in &session.messages {
                    render_message(message.role.clone(), &message.text);
                }
                for request in &session.pending_permissions {
                    render_permission_request(request);
                }
            }
        }
        StreamEventPayload::ConversationMessage { message } => {
            render_message(message.role.clone(), &message.text);
        }
        StreamEventPayload::PermissionRequested { request } => {
            render_permission_request(request);
        }
        StreamEventPayload::SessionClosed { reason, .. } => {
            println!("[status] session closed: {reason}");
        }
        StreamEventPayload::Status { message } => {
            println!("[status] {message}");
        }
    }
}

fn render_message(role: MessageRole, text: &str) {
    match role {
        MessageRole::User => println!("[user] {text}"),
        MessageRole::Assistant => println!("[assistant] {text}"),
    }
}

fn render_permission_request(request: &PermissionRequest) {
    println!("[permission {}] {}", request.request_id, request.summary);
}

pub(super) fn permission_decision_label(decision: &PermissionDecision) -> &'static str {
    match decision {
        PermissionDecision::Approve => "approved",
        PermissionDecision::Deny => "denied",
    }
}

fn render_initial_snapshot_delta(
    session: &SessionSnapshot,
    known_snapshot_state: &InitialSnapshotState,
) {
    for message in &session.messages {
        if !known_snapshot_state.message_ids.contains(&message.id) {
            render_message(message.role.clone(), &message.text);
        }
    }
    for request in &session.pending_permissions {
        if !known_snapshot_state
            .permission_request_ids
            .contains(&request.request_id)
        {
            render_permission_request(request);
        }
    }
}
