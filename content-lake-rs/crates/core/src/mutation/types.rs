//! Mutation type definitions matching Sanity's mutation protocol.
//!
//! Wire format (each element of the `mutations` array):
//!
//! ```json
//! {"create":            {"_id": "a", "_type": "t", ...}}
//! {"createOrReplace":   {"_id": "a", "_type": "t", ...}}
//! {"createIfNotExists": {"_id": "a", "_type": "t", ...}}
//! {"delete":            {"id": "a"}}
//! {"patch":             {"id": "a", "set": {...}, ...}}
//! ```
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A single Sanity mutation. Represented as an externally tagged enum so
/// that JSON `{"create": {...}}` deserializes into `Mutation::Create(...)`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Mutation {
    /// Create a document. Fails if one already exists with the given `_id`.
    Create(CreateMutation),
    /// Create or replace a document atomically.
    CreateOrReplace(CreateOrReplaceMutation),
    /// Create if the document doesn't yet exist; otherwise no-op.
    CreateIfNotExists(CreateIfNotExistsMutation),
    /// Delete a document by id.
    Delete(DeleteMutation),
    /// Patch operations applied to a document.
    Patch(Box<PatchMutation>),
}

/// The document payload for create-family mutations. The full document
/// (including `_id`, `_type`, and arbitrary user fields) is carried as a
/// JSON object.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CreateMutation {
    pub document: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CreateOrReplaceMutation {
    pub document: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CreateIfNotExistsMutation {
    pub document: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DeleteTarget {
    ById {
        id: String,
    },
    ByQuery {
        query: String,
        params: Option<Value>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteMutation {
    #[serde(flatten)]
    pub target: DeleteTarget,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchMutation {
    pub id: String,
    #[serde(default, rename = "ifRevisionID", skip_serializing_if = "Option::is_none")]
    pub if_revision_id: Option<String>,
    #[serde(flatten)]
    pub operations: PatchOperations,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchOperations {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set_if_missing: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unset: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inc: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dec: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub insert: Option<InsertOperation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff_match_patch: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InsertOperation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replace: Option<String>,
    pub items: Vec<Value>,
}

/// Result of a mutation transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MutationResponse {
    pub transaction_id: String,
    pub results: Vec<MutationResultSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MutationResultSummary {
    pub id: String,
    pub operation: String,
}
