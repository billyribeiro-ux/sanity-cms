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
}
