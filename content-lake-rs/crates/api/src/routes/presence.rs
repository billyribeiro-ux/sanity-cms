//! Presence (multi-user cursors) over WebSocket.
//!
//! `GET /v1/presence/{dataset}` upgrades to a WebSocket. Each connected
//! Studio session sends JSON messages describing what document + field it
//! is focused on; the server broadcasts those to every other session in
//! the same dataset.
//!
//! Wire format (subset of Sanity's presence protocol):
//!
//! Server → client (on connect):
//! ```json
//! {"type":"welcome","sessionId":"<uuid>"}
//! ```
//! ```json
//! {"type":"initial","sessions":[PresenceState, ...]}
//! ```
//!
//! Client → server (state update):
//! ```json
//! {"type":"state","documentId":"drafts.foo","selection":{...}}
//! ```
//!
//! Server → client (broadcast):
//! ```json
//! {"type":"state","sessionId":"...","userId":"...","userName":"...", ...}
//! {"type":"disappear","sessionId":"..."}
//! ```

use std::sync::{Arc, Mutex};

use axum::{
    Router,
    extract::{
        Path, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    response::Response,
    routing::get,
};
use chrono::Utc;
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use uuid::Uuid;

use crate::state::AppState;
use content_lake_core::{
    document::repo,
    events::types::{PresenceEvent, PresenceState},
};

pub fn routes() -> Router<AppState> {
    Router::new().route("/v1/presence/{dataset}", get(presence_ws))
}

/// Snapshot store of currently-active sessions. Populated from PresenceBus
/// events; serves as the "who's here right now" answer for new connections.
#[derive(Default, Clone)]
pub struct PresenceSnapshot {
    inner: Arc<Mutex<HashMap<String, PresenceState>>>,
}

impl PresenceSnapshot {
    pub fn update(&self, state: PresenceState) {
        let mut g = self.inner.lock().unwrap();
        g.insert(state.session_id.clone(), state);
    }
    pub fn remove(&self, session_id: &str) {
        let mut g = self.inner.lock().unwrap();
        g.remove(session_id);
    }
    pub fn snapshot(&self, dataset_id: Uuid) -> Vec<PresenceState> {
        let g = self.inner.lock().unwrap();
        g.values()
            .filter(|s| s.dataset_id == dataset_id)
            .cloned()
            .collect()
    }
}

// Lazy global store keyed per-process (MVP single-node).
use std::sync::OnceLock;
static SNAPSHOT: OnceLock<PresenceSnapshot> = OnceLock::new();
fn snap() -> &'static PresenceSnapshot {
    SNAPSHOT.get_or_init(PresenceSnapshot::default)
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum ClientMsg {
    /// A session update: focused document / selection.
    #[serde(rename = "state")]
    State {
        #[serde(default, rename = "documentId")]
        document_id: Option<String>,
        #[serde(default)]
        selection: Option<Value>,
    },
    /// Keepalive / heartbeat — client can send at will.
    #[serde(rename = "heartbeat")]
    Heartbeat,
}

async fn presence_ws(
    State(state): State<AppState>,
    Path(dataset): Path<String>,
    ws: WebSocketUpgrade,
) -> Response {
    ws.on_upgrade(move |socket| handle_ws(state, dataset, socket))
}

async fn handle_ws(state: AppState, dataset: String, socket: WebSocket) {
    let project = state.config().bootstrap_project.clone();

    // Resolve dataset. On error, close with a JSON error frame.
    let dataset_id = match repo::resolve_dataset(state.pool(), &project, &dataset).await {
        Ok(id) => id,
        Err(e) => {
            let mut sock = socket;
            let _ = sock
                .send(Message::Text(
                    json!({"type":"error","message":format!("{e}")}).to_string().into(),
                ))
                .await;
            let _ = sock.close().await;
            return;
        }
    };

    let session_id = Uuid::now_v7().as_simple().to_string();

    // Default identity — for MVP the auth middleware exempts this path; real
    // production setups should JWT-gate it and pull user info from Extensions.
    // For now, use a synthetic anonymous identity.
    let user_id = "anonymous".to_string();
    let user_name = "Anonymous".to_string();
    let user_email = "anonymous@localhost".to_string();

    let (mut tx, mut rx) = socket.split();

    // Send welcome + initial snapshot of who's currently here.
    let _ = tx
        .send(Message::Text(
            json!({"type":"welcome","sessionId":session_id}).to_string().into(),
        ))
        .await;
    let initial = snap().snapshot(dataset_id);
    let _ = tx
        .send(Message::Text(
            json!({"type":"initial","sessions":initial}).to_string().into(),
        ))
        .await;

    // Subscribe to the presence bus, filter by our dataset, relay to client.
    let mut bus_rx = state.presence_bus().subscribe();
    let my_session_id = session_id.clone();
    let relay_task = tokio::spawn(async move {
        loop {
            let ev = match bus_rx.recv().await {
                Ok(e) => e,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => return,
            };
            let skip = matches!(&ev, PresenceEvent::State(s)
                if s.session_id == my_session_id || s.dataset_id != dataset_id)
                || matches!(&ev, PresenceEvent::Disappear { session_id }
                    if session_id == &my_session_id);
            if skip {
                continue;
            }
            let Ok(payload) = serde_json::to_string(&ev) else {
                continue;
            };
            if tx.send(Message::Text(payload.into())).await.is_err() {
                return;
            }
        }
    });

    // Pump client messages. On state, update snapshot + broadcast.
    while let Some(msg) = rx.next().await {
        let Ok(m) = msg else { break };
        let text = match m {
            Message::Text(t) => t.to_string(),
            Message::Close(_) => break,
            Message::Ping(_) | Message::Pong(_) | Message::Binary(_) => continue,
        };
        let Ok(client) = serde_json::from_str::<ClientMsg>(&text) else {
            continue;
        };
        match client {
            ClientMsg::State { document_id, selection } => {
                let ps = PresenceState {
                    dataset_id,
                    session_id: session_id.clone(),
                    user_id: user_id.clone(),
                    user_name: user_name.clone(),
                    user_email: user_email.clone(),
                    document_id,
                    selection,
                    timestamp: Utc::now(),
                };
                snap().update(ps.clone());
                let _ = state.presence_bus().publish(PresenceEvent::State(ps));
            }
            ClientMsg::Heartbeat => { /* no-op */ }
        }
    }

    // Cleanup: remove from snapshot, broadcast disappear, tear down relay.
    snap().remove(&session_id);
    let _ = state
        .presence_bus()
        .publish(PresenceEvent::Disappear { session_id });
    relay_task.abort();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(id: &str, dataset: Uuid) -> PresenceState {
        PresenceState {
            dataset_id: dataset,
            session_id: id.to_string(),
            user_id: "u".into(),
            user_name: "Alice".into(),
            user_email: "a@b.c".into(),
            document_id: None,
            selection: None,
            timestamp: chrono::Utc::now(),
        }
    }

    #[test]
    fn snapshot_update_and_remove() {
        let snap = PresenceSnapshot::default();
        let ds = Uuid::new_v4();
        snap.update(state("s1", ds));
        snap.update(state("s2", ds));
        assert_eq!(snap.snapshot(ds).len(), 2);
        snap.remove("s1");
        assert_eq!(snap.snapshot(ds).len(), 1);
    }

    #[test]
    fn snapshot_filters_by_dataset() {
        let snap = PresenceSnapshot::default();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        snap.update(state("s1", a));
        snap.update(state("s2", b));
        assert_eq!(snap.snapshot(a).len(), 1);
        assert_eq!(snap.snapshot(b).len(), 1);
    }
}
