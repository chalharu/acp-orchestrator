use std::collections::HashSet;

use super::*;
use crate::contract_messages::ConversationMessage;
use crate::contract_permissions::PermissionRequest;
use eventsource_stream::Eventsource;
use futures_util::{StreamExt, pin_mut};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct InitialSnapshotState {
    message_ids: HashSet<String>,
    permission_request_ids: HashSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum StreamUpdate {
    ConversationMessage(ConversationMessage),
    PermissionRequested(PermissionRequest),
    SessionClosed { session_id: String, reason: String },
    Status(String),
}

impl InitialSnapshotState {
    pub(super) fn from_messages_and_permissions(
        messages: &[ConversationMessage],
        pending_permissions: &[PermissionRequest],
    ) -> Self {
        Self {
            message_ids: messages.iter().map(|message| message.id.clone()).collect(),
            permission_request_ids: pending_permissions
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
    stream_updates(
        client,
        events_url,
        auth_token,
        initial_snapshot_state,
        |update| render_update(&update),
    )
    .await
}

pub(super) async fn stream_updates<F>(
    client: Client,
    events_url: String,
    auth_token: String,
    initial_snapshot_state: Option<InitialSnapshotState>,
    mut handle_update: F,
) -> Result<()>
where
    F: FnMut(StreamUpdate),
{
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
        let payload = decode_stream_event(event)?;
        for update in stream_event_updates(payload, &mut initial_snapshot_state) {
            handle_update(update);
        }
    }

    Ok(())
}

#[cfg(test)]
pub(super) fn render_event(event: &StreamEvent) {
    for update in event_updates(event.clone()) {
        render_update(&update);
    }
}

pub(super) fn render_resume_state(
    messages: &[ConversationMessage],
    pending_permissions: &[PermissionRequest],
) {
    if messages.is_empty() && pending_permissions.is_empty() {
        println!("[status] session ready");
        return;
    }

    for update in resume_state_updates(messages, pending_permissions) {
        render_update(&update);
    }
}

pub(super) fn permission_decision_label(decision: &PermissionDecision) -> &'static str {
    match decision {
        PermissionDecision::Approve => "approved",
        PermissionDecision::Deny => "denied",
    }
}

fn render_update(update: &StreamUpdate) {
    match update {
        StreamUpdate::ConversationMessage(message) => {
            render_message(message.role.clone(), &message.text)
        }
        StreamUpdate::PermissionRequested(request) => render_permission_request(request),
        StreamUpdate::SessionClosed { reason, .. } => println!("[status] session closed: {reason}"),
        StreamUpdate::Status(message) => println!("[status] {message}"),
    }
}

fn decode_stream_event<E>(
    event: std::result::Result<eventsource_stream::Event, E>,
) -> Result<StreamEvent>
where
    E: std::error::Error + Send + Sync + 'static,
{
    let event = event.map_err(|source| CliError::ReadEventStream {
        source: Box::new(source),
    })?;
    serde_json::from_str(&event.data).context(DecodeStreamEventSnafu)
}

fn resume_state_updates(
    messages: &[ConversationMessage],
    pending_permissions: &[PermissionRequest],
) -> Vec<StreamUpdate> {
    messages
        .iter()
        .cloned()
        .map(StreamUpdate::ConversationMessage)
        .chain(
            pending_permissions
                .iter()
                .cloned()
                .map(StreamUpdate::PermissionRequested),
        )
        .collect()
}

fn event_updates(event: StreamEvent) -> Vec<StreamUpdate> {
    match event.payload {
        StreamEventPayload::SessionSnapshot { session } => {
            resume_state_updates(&session.messages, &session.pending_permissions)
        }
        StreamEventPayload::ConversationMessage { message } => {
            vec![StreamUpdate::ConversationMessage(message)]
        }
        StreamEventPayload::PermissionRequested { request } => {
            vec![StreamUpdate::PermissionRequested(request)]
        }
        StreamEventPayload::SessionClosed { session_id, reason } => {
            vec![StreamUpdate::SessionClosed { session_id, reason }]
        }
        StreamEventPayload::Status { message } => vec![StreamUpdate::Status(message)],
    }
}

fn stream_event_updates(
    payload: StreamEvent,
    initial_snapshot_state: &mut Option<InitialSnapshotState>,
) -> Vec<StreamUpdate> {
    match payload.payload {
        StreamEventPayload::SessionSnapshot { session } => match initial_snapshot_state.take() {
            Some(known_snapshot_state) => {
                initial_snapshot_delta_updates(&session, &known_snapshot_state)
            }
            None => resume_state_updates(&session.messages, &session.pending_permissions),
        },
        _ => event_updates(payload),
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

fn initial_snapshot_delta_updates(
    session: &SessionSnapshot,
    known_snapshot_state: &InitialSnapshotState,
) -> Vec<StreamUpdate> {
    let mut updates = Vec::new();
    for message in &session.messages {
        if !known_snapshot_state.message_ids.contains(&message.id) {
            updates.push(StreamUpdate::ConversationMessage(message.clone()));
        }
    }
    for request in &session.pending_permissions {
        if !known_snapshot_state
            .permission_request_ids
            .contains(&request.request_id)
        {
            updates.push(StreamUpdate::PermissionRequested(request.clone()));
        }
    }
    updates
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn created_at() -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0)
            .single()
            .expect("timestamp should be valid")
    }

    fn assistant_message(id: &str, text: &str) -> ConversationMessage {
        ConversationMessage {
            id: id.to_string(),
            role: MessageRole::Assistant,
            text: text.to_string(),
            created_at: created_at(),
        }
    }

    #[test]
    fn permission_decision_labels_cover_both_outcomes() {
        assert_eq!(
            permission_decision_label(&PermissionDecision::Approve),
            "approved"
        );
        assert_eq!(
            permission_decision_label(&PermissionDecision::Deny),
            "denied"
        );
    }

    #[test]
    fn render_resume_state_handles_empty_sessions() {
        render_resume_state(&[], &[]);
    }

    #[test]
    fn stream_event_updates_replay_full_snapshots_without_a_baseline() {
        let snapshot = SessionSnapshot {
            id: "s_test".to_string(),
            workspace_id: "w_test".to_string(),
            title: "New chat".to_string(),
            status: crate::contract_sessions::SessionStatus::Active,
            latest_sequence: 1,
            messages: vec![assistant_message("m_1", "hello")],
            pending_permissions: vec![PermissionRequest {
                request_id: "req_1".to_string(),
                summary: "read_text_file README.md".to_string(),
            }],
        };

        let updates = stream_event_updates(StreamEvent::snapshot(snapshot.clone()), &mut None);

        assert_eq!(
            updates,
            vec![
                StreamUpdate::ConversationMessage(snapshot.messages[0].clone()),
                StreamUpdate::PermissionRequested(snapshot.pending_permissions[0].clone()),
            ]
        );
    }

    #[test]
    fn initial_snapshot_delta_updates_skip_already_rendered_entries() {
        let session = SessionSnapshot {
            id: "s_test".to_string(),
            workspace_id: "w_test".to_string(),
            title: "New chat".to_string(),
            status: crate::contract_sessions::SessionStatus::Active,
            latest_sequence: 2,
            messages: vec![
                assistant_message("m_known", "known"),
                assistant_message("m_new", "new"),
            ],
            pending_permissions: vec![
                PermissionRequest {
                    request_id: "req_known".to_string(),
                    summary: "known".to_string(),
                },
                PermissionRequest {
                    request_id: "req_new".to_string(),
                    summary: "new".to_string(),
                },
            ],
        };
        let known_snapshot_state = InitialSnapshotState::from_messages_and_permissions(
            &[assistant_message("m_known", "known")],
            &[PermissionRequest {
                request_id: "req_known".to_string(),
                summary: "known".to_string(),
            }],
        );

        assert_eq!(
            initial_snapshot_delta_updates(&session, &known_snapshot_state),
            vec![
                StreamUpdate::ConversationMessage(session.messages[1].clone()),
                StreamUpdate::PermissionRequested(session.pending_permissions[1].clone()),
            ]
        );
    }
}
