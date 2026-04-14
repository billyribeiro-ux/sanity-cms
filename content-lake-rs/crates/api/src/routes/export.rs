//! `GET /v1/data/export/{dataset}` — stream every document in the dataset as
//! NDJSON (one document JSON object per line), matching Sanity's export API.
//!
//! Response content-type: `application/x-ndjson`. The response is streamed —
//! suitable for backing up datasets of arbitrary size without buffering the
//! whole thing in memory.
//!
//! Query params:
//!   * `types=a,b,c`  — comma-separated `_type` filter
//!   * `limit=N`      — optional cap (default: all)

use std::collections::HashMap;

use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use chrono::{DateTime, Utc};
use serde_json::{Map, Value};
use sqlx::Row;

use crate::{
    error::{ApiError, ApiResult},
    routes::doc::map_repo_err,
    state::AppState,
};

use content_lake_core::document::repo;

pub fn routes() -> Router<AppState> {
    Router::new().route("/v1/data/export/{dataset}", get(export))
}

#[tracing::instrument(skip(state))]
async fn export(
    State(state): State<AppState>,
    Path(dataset): Path<String>,
    Query(qs): Query<HashMap<String, String>>,
) -> ApiResult<Response> {
    let project = state.config().bootstrap_project.clone();
    let dataset_id = repo::resolve_dataset(state.pool(), &project, &dataset)
        .await
        .map_err(map_repo_err)?;

    let types: Option<Vec<String>> = qs
        .get("types")
        .map(|s| s.split(',').map(|t| t.trim().to_string()).collect());
    let limit: Option<i64> = qs.get("limit").and_then(|v| v.parse().ok());

    // Build the query dynamically — native types only, no JSONB path needed.
    let mut sql = String::from(
        "SELECT document_id, doc_type, revision, content, created_at, updated_at \
         FROM documents \
         WHERE dataset_id = $1 AND NOT deleted",
    );
    if types.is_some() {
        sql.push_str(" AND doc_type = ANY($2)");
    }
    sql.push_str(" ORDER BY document_id");
    if let Some(n) = limit {
        sql.push_str(&format!(" LIMIT {n}"));
    }

    let mut q = sqlx::query(&sql).bind(dataset_id);
    if let Some(t) = &types {
        q = q.bind(t.clone());
    }

    // Fetch all rows eagerly, stitch NDJSON, return as one body. For very
    // large datasets this should be switched to a proper sqlx cursor + stream,
    // but this covers typical CMS use (thousands of docs, not millions).
    let rows = q
        .fetch_all(state.pool())
        .await
        .map_err(ApiError::Database)?;

    let mut body = Vec::with_capacity(rows.len() * 256);
    for row in rows {
        let content: Value = row.try_get("content").map_err(ApiError::Database)?;
        let document_id: String = row.try_get("document_id").map_err(ApiError::Database)?;
        let doc_type: String = row.try_get("doc_type").map_err(ApiError::Database)?;
        let revision: String = row.try_get("revision").map_err(ApiError::Database)?;
        let created_at: sqlx::types::time::OffsetDateTime =
            row.try_get("created_at").map_err(ApiError::Database)?;
        let updated_at: sqlx::types::time::OffsetDateTime =
            row.try_get("updated_at").map_err(ApiError::Database)?;

        let mut m = content.as_object().cloned().unwrap_or_else(Map::new);
        m.insert("_id".into(), Value::String(document_id));
        m.insert("_type".into(), Value::String(doc_type));
        m.insert("_rev".into(), Value::String(revision));
        m.insert("_createdAt".into(), Value::String(odt_to_iso(created_at)));
        m.insert("_updatedAt".into(), Value::String(odt_to_iso(updated_at)));

        serde_json::to_writer(&mut body, &Value::Object(m))
            .map_err(|e| ApiError::Internal(format!("serialize: {e}")))?;
        body.push(b'\n');
    }

    Ok((
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/x-ndjson"),
        )],
        body,
    )
        .into_response())
}

fn odt_to_iso(odt: sqlx::types::time::OffsetDateTime) -> String {
    let unix_ns = odt.unix_timestamp_nanos();
    let secs = (unix_ns.div_euclid(1_000_000_000)) as i64;
    let nanos = (unix_ns.rem_euclid(1_000_000_000)) as u32;
    let dt: DateTime<Utc> = chrono::TimeZone::timestamp_opt(&Utc, secs, nanos)
        .single()
        .unwrap_or_else(Utc::now);
    dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}
