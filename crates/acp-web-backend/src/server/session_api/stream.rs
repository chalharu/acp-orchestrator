use std::{convert::Infallible, sync::Arc, time::Duration};

use axum::{
    extract::{Extension, Path, State},
    response::sse::{Event, KeepAlive, Sse},
};
use futures_util::{Stream, StreamExt, stream};
use tokio_stream::wrappers::{BroadcastStream, errors::BroadcastStreamRecvError};

use crate::{auth::AuthenticatedPrincipal, contract_stream::StreamEvent, sessions::SessionStore};

use super::super::{AppError, AppState};

pub(in crate::server) async fn stream_session_events(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let owner = state.owner_context(principal).await?;
    let (snapshot, receiver) = state
        .store
        .session_events(&owner.live_owner_id, &session_id)
        .await?;

    let initial_event = stream::once(async move {
        Ok::<Event, Infallible>(to_sse_event(StreamEvent::snapshot(snapshot)))
    });
    let store = state.store.clone();
    let owner_id = owner.live_owner_id;
    let updates = BroadcastStream::new(receiver)
        .then(move |result| {
            let store = store.clone();
            let owner_id = owner_id.clone();
            let session_id = session_id.clone();
            async move {
                stream_event_from_receiver_result(result, store, owner_id, session_id)
                    .await
                    .map(to_sse_event)
                    .map(Ok)
            }
        })
        .filter_map(|event| async move { event });

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

async fn stream_event_from_receiver_result(
    result: Result<StreamEvent, BroadcastStreamRecvError>,
    store: Arc<SessionStore>,
    owner_id: String,
    session_id: String,
) -> Option<StreamEvent> {
    match result {
        Ok(event) => Some(event),
        Err(BroadcastStreamRecvError::Lagged(_)) => store
            .session_snapshot(&owner_id, &session_id)
            .await
            .ok()
            .map(StreamEvent::snapshot),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::SessionStore;

    #[tokio::test]
    async fn stream_receiver_results_forward_regular_events() {
        let store = Arc::new(SessionStore::new(4));
        let event = StreamEvent::status(7, "still connected");

        let recovered = stream_event_from_receiver_result(
            Ok(event.clone()),
            store,
            "alice".to_string(),
            "s_missing".to_string(),
        )
        .await;

        assert_eq!(recovered, Some(event));
    }

    #[tokio::test]
    async fn lagged_stream_receiver_results_recover_with_snapshot() {
        let store = Arc::new(SessionStore::new(4));
        let session = store
            .create_session("alice", "w_test")
            .await
            .expect("session creation should succeed");

        let recovered = stream_event_from_receiver_result(
            Err(BroadcastStreamRecvError::Lagged(3)),
            store,
            "alice".to_string(),
            session.id.clone(),
        )
        .await
        .expect("lagged stream should recover with a session snapshot");

        assert!(matches!(
            recovered.payload,
            crate::contract_stream::StreamEventPayload::SessionSnapshot { session: snapshot }
                if snapshot.id == session.id
        ));
    }
}
