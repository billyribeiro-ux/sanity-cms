/// Mutation type definitions matching Sanity's mutation protocol.
/// Executor will be implemented in Phase 1.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Mutation {
    Create(CreateMutation),
    CreateOrReplace(CreateOrReplaceMutation),
    CreateIfNotExists(CreateIfNotExistsMutation),
    Delete(DeleteMutation),
    Patch(PatchMutation),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateMutation {
    pub document: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateOrReplaceMutation {
    pub document: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub if_revision_id: Option<String>,
    #[serde(flatten)]
    pub operations: PatchOperations,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchOperations {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub set: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub set_if_missing: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merge: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unset: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inc: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dec: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insert: Option<InsertOperation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff_match_patch: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InsertOperation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replace: Option<String>,
    pub items: Vec<Value>,
}

/// Result of a mutation transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MutationResponse {
    pub transaction_id: String,
    pub results: Vec<MutationResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MutationResult {
    pub id: String,
    pub operation: String,
}
