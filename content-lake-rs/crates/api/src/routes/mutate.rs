//! `POST /v1/data/mutate/{dataset}` — apply a batch of mutations.
//!
//! Contract (Sanity-compatible):
//!
//! Request body:
//!   ```json
//!   {
//!     "mutations": [{"create": {...}}, {"patch": {...}}, ...],
//!     "transactionId": "optional-uuid",
//!     "visibility": "sync" | "async" | "deferred"
//!   }
//!   ```
//!
//! Response:
//!   ```json
//!   {
//!     "transactionId": "uuid",
//!     "results": [{"id": "...", "operation": "create|update|delete|none"}]
//!   }
//!   ```
//!
//! All mutations in a request apply inside a single Postgres transaction.
//! A single failure rolls the whole batch back.

use axum::{
    Router,
    extract::{Path, State},
    response::Json,
    routing::post,
};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::Acquire;
use uuid::Uuid;

use crate::{
    error::{ApiError, ApiResult},
    routes::doc::map_repo_err,
    state::AppState,
};

use content_lake_core::{
    document::repo,
    mutation::{
        engine::{self, MutationResult},
        types::{DeleteTarget, Mutation},
    },
};

pub fn routes() -> Router<AppState> {
    Router::new().route("/v1/data/mutate/{dataset}", post(mutate))
}

#[derive(Debug, Deserialize)]
struct MutateRequest {
    mutations: Vec<Mutation>,
    #[serde(default, rename = "transactionId")]
    transaction_id: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    visibility: Option<String>,
}

#[tracing::instrument(skip(state, body))]
async fn mutate(
    State(state): State<AppState>,
    Path(dataset): Path<String>,
    Json(body): Json<MutateRequest>,
) -> ApiResult<Json<Value>> {
    let project = state.config().bootstrap_project.clone();
    let dataset_id = repo::resolve_dataset(state.pool(), &project, &dataset)
        .await
        .map_err(map_repo_err)?;

    let tx_id = body
        .transaction_id
        .clone()
        .unwrap_or_else(|| Uuid::now_v7().as_simple().to_string());

    let mut conn = state.pool().acquire().await.map_err(ApiError::Database)?;
    let mut tx = conn.begin().await.map_err(ApiError::Database)?;

    let mut results = Vec::with_capacity(body.mutations.len());

    for mutation in &body.mutations {
        let target_id = extract_target_id(mutation);
        let current = if let Some(id) = &target_id {
            repo::get_document(&mut *tx, dataset_id, id)
                .await
                .map_err(map_repo_err)?
        } else {
            None
        };

        let new_rev = new_rev();
        let outcome = engine::apply_mutation(current.as_ref(), mutation, &new_rev, Utc::now())
            .map_err(map_engine_err)?;

        let (id, op) = match outcome {
            MutationResult::Created(doc) => {
                let doc_id = doc._id.clone();
                repo::insert_document(
                    &mut *tx,
                    dataset_id,
                    &doc._id,
                    &doc._type,
                    Some(&doc._rev),
                    &Value::Object(doc.content),
                )
                .await
                .map_err(map_repo_err)?;
                (doc_id, "create")
            }
            MutationResult::Updated(doc) => {
                let doc_id = doc._id.clone();
                repo::upsert_document(
                    &mut *tx,
                    dataset_id,
                    &doc._id,
                    &doc._type,
                    Some(&doc._rev),
                    &Value::Object(doc.content),
                )
                .await
                .map_err(map_repo_err)?;
                (doc_id, "update")
            }
            MutationResult::Deleted(id) => {
                repo::delete_document(&mut *tx, dataset_id, &id)
                    .await
                    .map_err(map_repo_err)?;
                (id, "delete")
            }
            MutationResult::NoOp => {
                // createIfNotExists when the doc already existed — report id but no-op.
                let id = target_id.clone().unwrap_or_default();
                (id, "none")
            }
        };

        results.push(json!({"id": id, "operation": op}));
    }

    tx.commit().await.map_err(ApiError::Database)?;

    Ok(Json(json!({
        "transactionId": tx_id,
        "results": results,
    })))
}

/// Pull the document id that a mutation targets, so we can fetch the current
/// state before applying. Returns `None` for mutations that don't specify
/// a pre-existing id (e.g. `create` without `_id`).
fn extract_target_id(m: &Mutation) -> Option<String> {
    match m {
        Mutation::Create(c) => c.document.get("_id").and_then(|v| v.as_str()).map(String::from),
        Mutation::CreateOrReplace(c) => {
            c.document.get("_id").and_then(|v| v.as_str()).map(String::from)
        }
        Mutation::CreateIfNotExists(c) => {
            c.document.get("_id").and_then(|v| v.as_str()).map(String::from)
        }
        Mutation::Delete(d) => match &d.target {
            DeleteTarget::ById { id } => Some(id.clone()),
            DeleteTarget::ByQuery { .. } => None,
        },
        Mutation::Patch(p) => Some(p.id.clone()),
    }
}

fn new_rev() -> String {
    let s = Uuid::now_v7().as_simple().to_string();
    s[..16].to_string()
}

fn map_engine_err(err: engine::MutationError) -> ApiError {
    use engine::MutationError as E;
    match err {
        E::AlreadyExists(m) => ApiError::Conflict(m),
        E::NotFound(m) => ApiError::NotFound(m),
        E::RevisionMismatch { expected, actual } => ApiError::Conflict(format!(
            "revision mismatch: expected {expected}, actual {actual}"
        )),
        E::InvalidPath(m) => ApiError::BadRequest(format!("invalid path: {m}")),
        E::InvalidOperation(m) => ApiError::BadRequest(m),
        E::Unsupported(m) => ApiError::BadRequest(format!("unsupported: {m}")),
    }
}
