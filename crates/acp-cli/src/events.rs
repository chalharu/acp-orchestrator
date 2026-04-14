use super::*;
use eventsource_stream::Eventsource;
use futures_util::{StreamExt, pin_mut};

pub(super) async fn stream_events_to_stderr(
    client: Client,
    events_url: String,
    auth_token: String,
) {
    if let Err(error) = stream_events(client, events_url, auth_token).await {
        eprintln!("[status] event stream ended: {error}");
    }
}

pub(super) async fn stream_events(
    client: Client,
    events_url: String,
    auth_token: String,
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

    while let Some(event) = stream.next().await {
        let event = event.map_err(|source| CliError::ReadEventStream {
            source: Box::new(source),
        })?;
        let payload: StreamEvent =
            serde_json::from_str(&event.data).context(DecodeStreamEventSnafu)?;
        render_event(&payload);
    }

    Ok(())
}

pub(super) fn render_event(event: &StreamEvent) {
    match &event.payload {
        StreamEventPayload::SessionSnapshot { session } => {
            if session.messages.is_empty() {
                println!("[status] session ready");
            } else {
                for message in &session.messages {
                    render_message(message.role.clone(), &message.text);
                }
            }
        }
        StreamEventPayload::ConversationMessage { message } => {
            render_message(message.role.clone(), &message.text);
        }
        StreamEventPayload::PermissionRequested { request } => {
            println!("[permission {}] {}", request.request_id, request.summary);
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

pub(super) fn permission_decision_label(decision: &PermissionDecision) -> &'static str {
    match decision {
        PermissionDecision::Approve => "approved",
        PermissionDecision::Deny => "denied",
    }
}
