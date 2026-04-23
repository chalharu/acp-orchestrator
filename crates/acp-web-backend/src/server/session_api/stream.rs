use std::{convert::Infallible, time::Duration};

use axum::{
    extract::{Extension, Path, State},
    response::sse::{Event, KeepAlive, Sse},
};
use futures_util::{Stream, StreamExt, stream};
use tokio_stream::wrappers::BroadcastStream;

use crate::{auth::AuthenticatedPrincipal, contract_stream::StreamEvent};

use super::super::{AppError, AppState};

pub(in crate::server) async fn stream_session_events(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let owner = state.owner_context(principal).await?;
    let (snapshot, receiver) = state
        .store
        .session_events(&owner.principal.id, &session_id)
        .await?;

    let initial_event = stream::once(async move {
        Ok::<Event, Infallible>(to_sse_event(StreamEvent::snapshot(snapshot)))
    });
    let updates = BroadcastStream::new(receiver)
        .filter_map(|result| async move { result.ok().map(to_sse_event).map(Ok) });

    Ok(Sse::new(initial_event.chain(updates)).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    ))
}

fn to_sse_event(event: StreamEvent) -> Event {
    let sequence = event.sequence.to_string();
    let payload =
        serde_json::to_string(&event).expect("stream events should always serialize successfully");

    Event::default()
        .event(event.event_name())
        .id(sequence)
        .data(payload)
}
