use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// Core Sanity document stored in the content lake.
/// Maps to the `documents` PostgreSQL table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SanityDocument {
    pub _id: String,
    pub _type: String,
    #[serde(rename = "_createdAt")]
    pub created_at: DateTime<Utc>,
    #[serde(rename = "_updatedAt")]
    pub updated_at: DateTime<Utc>,
    pub _rev: String,
    /// Arbitrary document fields stored as JSONB.
    #[serde(flatten)]
    pub content: serde_json::Map<String, Value>,
}

/// Database row representation of a document.
#[derive(Debug, Clone)]
pub struct DocumentRow {
    pub id: Uuid,
    pub dataset_id: Uuid,
    pub document_id: String,
    pub doc_type: String,
    pub revision: String,
    pub content: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted: bool,
}
