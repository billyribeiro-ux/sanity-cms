//! `GET /v1/data/listen/{dataset}` — Sanity-compatible SSE listen endpoint.
//!
//! Studio opens an EventSource to this URL and expects a `text/event-stream`.
//! We emit:
//!   * `welcome` once on connect with a listener id.
//!   * `mutation` per committed mutation touching the dataset.
//!   * `reconnect` when the broadcast channel lags and the client should
//!     reconnect to rebuild state.
//!
//! Keepalive comments are emitted every 15s so intermediaries don't kill the
//! idle connection.

use std::{convert::Infallible, time::Duration};

use axum::{
    Router,
    extract::{Path, State},
    response::{
        Sse,
        sse::{Event, KeepAlive},
    },
    routing::get,
};
use futures::stream::{self, Stream, StreamExt};
use serde_json::json;
use tokio_stream::wrappers::{BroadcastStream, errors::BroadcastStreamRecvError};
use uuid::Uuid;

use crate::{
    error::ApiResult,
    routes::doc::map_repo_err,
    state::AppState,
};

use content_lake_core::{
    document::repo,
    events::types::ContentLakeEvent,
};

pub fn routes() -> Router<AppState> {
    Router::new().route("/v1/data/listen/{dataset}", get(listen))
}

#[tracing::instrument(skip(state))]
async fn listen(
    State(state): State<AppState>,
    Path(dataset): Path<String>,
) -> ApiResult<Sse<impl Stream<Item = Result<Event, Infallible>>>> {
    let project = state.config().bootstrap_project.clone();
    let dataset_id = repo::resolve_dataset(state.pool(), &project, &dataset)
        .await
        .map_err(map_repo_err)?;

    let rx = state.event_bus().subscribe();
    let listener_id = Uuid::now_v7();

    // Initial `welcome` event.
    let welcome = Event::default()
        .event("welcome")
        .data(json!({ "listenerName": listener_id.to_string() }).to_string());
    let welcome_stream = stream::once(async move { Ok::<_, Infallible>(welcome) });

    // Map broadcast events into SSE `mutation` / `reconnect` events.
    let bus_stream = BroadcastStream::new(rx).filter_map(move |item| {
        let value = match item {
            Ok(ContentLakeEvent::DocumentMutation(ev)) if ev.dataset_id == dataset_id => {
                let payload = json!({
                    "documentId": ev.document_id,
                    "transactionId": ev.transaction_id,
                    "transition": ev.transition,
                    "mutations": ev.mutations,
                    "result": ev.result,
                });
                Some(
                    Event::default()
                        .event("mutation")
                        .data(payload.to_string()),
                )
            }
            Ok(_) => None,
            Err(BroadcastStreamRecvError::Lagged(n)) => Some(
                Event::default()
                    .event("reconnect")
                    .data(
                        json!({ "reason": format!("listener lagged by {n} events") })
                            .to_string(),
                    ),
            ),
        };
        async move { value.map(Ok::<_, Infallible>) }
    });

    let combined = welcome_stream.chain(bus_stream);

    Ok(Sse::new(combined).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))))
}
