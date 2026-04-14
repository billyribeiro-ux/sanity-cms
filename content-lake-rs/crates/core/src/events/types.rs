use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Events emitted after successful mutations, consumed by SSE listeners.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ContentLakeEvent {
    Welcome,
    Mutation(MutationEvent),
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
