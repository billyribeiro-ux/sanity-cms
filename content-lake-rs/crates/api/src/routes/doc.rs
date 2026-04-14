//! `GET /v1/data/doc/{dataset}/{ids}` — fetch one or more documents by id.
//!
//! Sanity's convention: `ids` is a comma-separated list. Missing documents
//! are surfaced in the `omitted` array rather than causing a 404.

use axum::{
    extract::{Path, State},
    response::Json,
    routing::get,
    Router,
};
use serde_json::{json, Value};

use crate::{
    error::{ApiError, ApiResult},
    state::AppState,
};

use content_lake_core::document::repo;

pub fn routes() -> Router<AppState> {
    Router::new().route("/v1/data/doc/{dataset}/{ids}", get(get_documents))
}

#[tracing::instrument(skip(state))]
async fn get_documents(
    State(state): State<AppState>,
    Path((dataset, ids)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    let project = state.config().bootstrap_project.clone();
    let dataset_id = repo::resolve_dataset(state.pool(), &project, &dataset)
        .await
        .map_err(map_repo_err)?;

    let id_list: Vec<String> = ids
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    if id_list.is_empty() {
        return Err(ApiError::BadRequest("no document ids in path".into()));
    }

    let (docs, omitted) = repo::get_documents(state.pool(), dataset_id, &id_list)
        .await
        .map_err(map_repo_err)?;

    let docs_json: Vec<Value> = docs
        .into_iter()
        .map(document_to_wire)
        .collect::<Result<_, _>>()
        .map_err(|e| ApiError::Internal(format!("document serialization: {e}")))?;

    let omitted_json: Vec<Value> = omitted
        .into_iter()
        .map(|(id, reason)| json!({"id": id, "reason": reason}))
        .collect();

    Ok(Json(json!({
        "documents": docs_json,
        "omitted": omitted_json,
    })))
}

/// Convert a `SanityDocument` into the wire JSON shape Studio expects:
/// system fields (`_id`, `_type`, `_rev`, `_createdAt`, `_updatedAt`) at the
/// top level alongside user content.
pub fn document_to_wire(
    d: content_lake_core::document::model::SanityDocument,
) -> Result<Value, serde_json::Error> {
    let mut m = d.content;
    m.insert("_id".into(), Value::String(d._id));
    m.insert("_type".into(), Value::String(d._type));
    m.insert("_rev".into(), Value::String(d._rev));
    m.insert(
        "_createdAt".into(),
        Value::String(
            d.created_at
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        ),
    );
    m.insert(
        "_updatedAt".into(),
        Value::String(
            d.updated_at
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        ),
    );
    Ok(Value::Object(m))
}

pub fn map_repo_err(err: repo::RepoError) -> ApiError {
    match err {
        repo::RepoError::NotFound(msg) => ApiError::NotFound(msg),
        repo::RepoError::AlreadyExists(msg) => ApiError::Conflict(msg),
        repo::RepoError::Database(e) => ApiError::Database(e),
    }
}
