use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// Events emitted after successful mutations, consumed by SSE listeners.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ContentLakeEvent {
    Welcome,
    Mutation(MutationEvent),
    /// Full document mutation event carrying the resulting document and
    /// enough metadata for Sanity-compatible SSE `mutation` events.
    DocumentMutation(DocumentMutationEvent),
    Reconnect,
}

impl ContentLakeEvent {
    /// Return the cluster-local origin of this event, if it has one. Used by
    /// the LISTEN bridge to drop echoes of events we already broadcast
    /// in-process.
    pub fn origin_node_id(&self) -> Option<Uuid> {
        match self {
            ContentLakeEvent::DocumentMutation(ev) => Some(ev.node_id),
            ContentLakeEvent::Mutation(_)
            | ContentLakeEvent::Welcome
            | ContentLakeEvent::Reconnect => None,
        }
    }
}

impl PresenceEvent {
    /// Return the cluster-local origin of this presence event, if it has one.
    pub fn origin_node_id(&self) -> Option<Uuid> {
        match self {
            PresenceEvent::State(s) => Some(s.node_id),
            PresenceEvent::Disappear { .. } => None,
        }
    }
}

/// A presence (multi-user cursor) update broadcast to all connected Studio
/// sessions in the same dataset. One of `state` (session announced or moved)
/// or `disappear` (session left).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PresenceEvent {
    /// A session is active and at a given position.
    #[serde(rename = "state")]
    State(PresenceState),
    /// A session has disconnected.
    #[serde(rename = "disappear")]
    Disappear { session_id: String },
}

/// Snapshot of where a user is currently focused in the Studio.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresenceState {
    pub dataset_id: Uuid,
    pub session_id: String,
    pub user_id: String,
    pub user_name: String,
    pub user_email: String,
    /// Optional: which document the user is viewing/editing.
    pub document_id: Option<String>,
    /// Opaque JSON selection payload from the client (path + selection type).
    pub selection: Option<Value>,
    pub timestamp: DateTime<Utc>,
    /// Cluster-local origin identifier. Each API node generates a random
    /// `node_id` at startup; when forwarded over Postgres LISTEN/NOTIFY, the
    /// receiving node uses this to drop its own echo (the event was already
    /// broadcast locally when it was published).
    #[serde(default)]
    pub node_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MutationEvent {
    pub dataset_id: String,
    pub document_id: String,
    pub transaction_id: String,
    pub previous_rev: Option<String>,
    pub result_rev: String,
    pub timestamp: DateTime<Utc>,
    pub effects: Option<serde_json::Value>,
    pub transaction_total_events: u32,
    pub transaction_current_event: u32,
}

/// A richer mutation event carrying the post-mutation result document and
/// the raw mutations, so SSE listeners can surface a Sanity-compatible
/// `mutation` payload to Studio.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentMutationEvent {
    pub dataset_id: Uuid,
    pub document_id: String,
    pub transaction_id: String,
    /// One of `"appear"`, `"update"`, `"disappear"`.
    pub transition: String,
    pub mutations: Value,
    pub result: Option<Value>,
    /// Cluster-local origin identifier. See `PresenceState::node_id`.
    #[serde(default)]
    pub node_id: Uuid,
}
