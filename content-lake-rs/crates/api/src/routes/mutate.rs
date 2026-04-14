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
    extract::{Path, State},
    response::Json,
    routing::post,
    Router,
};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Acquire;
use uuid::Uuid;

use crate::{
    error::{ApiError, ApiResult},
    routes::doc::map_repo_err,
    state::AppState,
};

use content_lake_core::{
    document::repo,
    events::types::{ContentLakeEvent, DocumentMutationEvent},
    mutation::{
        engine::{self, MutationResult},
        types::{DeleteTarget, Mutation},
    },
};

use crate::routes::doc::document_to_wire;

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
    // Events to broadcast after the transaction commits. We only publish on
    // success so subscribers don't see rolled-back mutations.
    let mut pending_events: Vec<DocumentMutationEvent> = Vec::with_capacity(body.mutations.len());
    // (document_id, previous_rev, result_rev) tuples for the transaction log.
    let mut touched: Vec<(String, Option<String>, Option<String>)> =
        Vec::with_capacity(body.mutations.len());
    // Raw mutation JSON captured for the transaction log.
    let mut raw_mutations: Vec<Value> = Vec::with_capacity(body.mutations.len());

    for mutation in &body.mutations {
        let target_id = extract_target_id(mutation);
        let current = if let Some(id) = &target_id {
            repo::get_document(&mut *tx, dataset_id, id)
                .await
                .map_err(map_repo_err)?
        } else {
            None
        };
        let previous_rev = current.as_ref().map(|d| d._rev.clone());

        let new_rev = new_rev();
        let outcome = engine::apply_mutation(current.as_ref(), mutation, &new_rev, Utc::now())
            .map_err(map_engine_err)?;

        let mutation_value = serde_json::to_value(mutation).unwrap_or(Value::Null);
        raw_mutations.push(mutation_value.clone());

        let (id, op, event) = match outcome {
            MutationResult::Created(doc) => {
                // Validate the finished document against the schema registry
                // BEFORE writing. An empty registry short-circuits to Ok.
                validate_doc(&state, &doc)?;
                let doc_id = doc._id.clone();
                let inserted = repo::insert_document(
                    &mut *tx,
                    dataset_id,
                    &doc._id,
                    &doc._type,
                    Some(&doc._rev),
                    &Value::Object(doc.content),
                )
                .await
                .map_err(map_repo_err)?;
                let result = document_to_wire(inserted).ok();
                let ev = DocumentMutationEvent {
                    dataset_id,
                    document_id: doc_id.clone(),
                    transaction_id: tx_id.clone(),
                    transition: "appear".to_string(),
                    mutations: json!([mutation_value.clone()]),
                    result,
                    node_id: state.node_id(),
                };
                touched.push((doc_id.clone(), None, Some(new_rev.clone())));
                (doc_id, "create", Some(ev))
            }
            MutationResult::Updated(doc) => {
                validate_doc(&state, &doc)?;
                let doc_id = doc._id.clone();
                let upserted = repo::upsert_document(
                    &mut *tx,
                    dataset_id,
                    &doc._id,
                    &doc._type,
                    Some(&doc._rev),
                    &Value::Object(doc.content),
                )
                .await
                .map_err(map_repo_err)?;
                let result = document_to_wire(upserted).ok();
                let ev = DocumentMutationEvent {
                    dataset_id,
                    document_id: doc_id.clone(),
                    transaction_id: tx_id.clone(),
                    transition: "update".to_string(),
                    mutations: json!([mutation_value.clone()]),
                    result,
                    node_id: state.node_id(),
                };
                touched.push((doc_id.clone(), previous_rev.clone(), Some(new_rev.clone())));
                (doc_id, "update", Some(ev))
            }
            MutationResult::Deleted(id) => {
                repo::delete_document(&mut *tx, dataset_id, &id)
                    .await
                    .map_err(map_repo_err)?;
                let ev = DocumentMutationEvent {
                    dataset_id,
                    document_id: id.clone(),
                    transaction_id: tx_id.clone(),
                    transition: "disappear".to_string(),
                    mutations: json!([mutation_value.clone()]),
                    result: None,
                    node_id: state.node_id(),
                };
                touched.push((id.clone(), previous_rev.clone(), None));
                (id, "delete", Some(ev))
            }
            MutationResult::NoOp => {
                // createIfNotExists when the doc already existed — report id but no-op.
                let id = target_id.clone().unwrap_or_default();
                (id, "none", None)
            }
        };

        if let Some(ev) = event {
            pending_events.push(ev);
        }
        results.push(json!({"id": id, "operation": op}));
    }

    // Append the transaction row + per-document rev history before commit so
    // the /v1/history endpoint has something to return.
    if !touched.is_empty() {
        repo::append_transaction(
            &mut tx,
            dataset_id,
            &tx_id,
            None, // TODO: populate author from CurrentUser extension
            &Value::Array(raw_mutations),
            &touched,
        )
        .await
        .map_err(map_repo_err)?;
    }

    tx.commit().await.map_err(ApiError::Database)?;

    // Broadcast after commit. Use the durable path so other cluster nodes
    // see the event via Postgres LISTEN/NOTIFY as well as our own SSE
    // subscribers. NOTIFY errors are logged but never fail the request —
    // the mutation has already committed.
    let bus = state.event_bus();
    for ev in pending_events {
        if let Err(e) = bus
            .publish_durable(state.pool(), ContentLakeEvent::DocumentMutation(ev))
            .await
        {
            tracing::warn!(error = %e, "durable event publish failed (mutation already committed)");
        }
    }

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
        Mutation::Create(c) => c
            .document
            .get("_id")
            .and_then(|v| v.as_str())
            .map(String::from),
        Mutation::CreateOrReplace(c) => c
            .document
            .get("_id")
            .and_then(|v| v.as_str())
            .map(String::from),
        Mutation::CreateIfNotExists(c) => c
            .document
            .get("_id")
            .and_then(|v| v.as_str())
            .map(String::from),
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

fn validate_doc(
    state: &AppState,
    doc: &content_lake_core::document::model::SanityDocument,
) -> ApiResult<()> {
    if state.schema_registry().is_empty() {
        return Ok(());
    }
    // Materialize the document into wire-shape JSON before validating so the
    // schema can inspect `_id`, `_type`, and user content together.
    let mut m = doc.content.clone();
    m.insert("_id".into(), Value::String(doc._id.clone()));
    m.insert("_type".into(), Value::String(doc._type.clone()));
    let wire = Value::Object(m);
    state
        .schema_registry()
        .validate_document(&wire)
        .map_err(|e| ApiError::BadRequest(format!("schema validation failed: {e}")))
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
