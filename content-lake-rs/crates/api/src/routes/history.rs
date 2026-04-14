//! `GET /v1/history/{dataset}/{documentId}` — return the mutation log for a
//! single document, newest first.
//!
//! Response shape (compatible with Sanity's history API for the fields the
//! Studio actually consumes):
//!
//! ```json
//! {
//!   "transactions": [
//!     {
//!       "id": "<tx-uuid>",
//!       "timestamp": "2026-04-14T12:34:56.789Z",
//!       "author": "p-admin",
//!       "documentIDs": ["foo"],
//!       "mutations": [...],
//!       "resultRev": "<short rev>",
//!       "previousRev": "<short rev or null>"
//!     }
//!   ]
//! }
//! ```

use axum::{
    extract::{Path, Query, State},
    routing::get,
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;

use crate::{
    error::{ApiError, ApiResult},
    routes::doc::map_repo_err,
    state::AppState,
};

use content_lake_core::document::repo;

pub fn routes() -> Router<AppState> {
    Router::new().route("/v1/history/{dataset}/{document_id}", get(history))
}

#[derive(Debug, Deserialize)]
struct HistoryQuery {
    /// Max number of transactions to return. Default 100, hard cap 1000.
    #[serde(default)]
    limit: Option<i64>,
}

#[tracing::instrument(skip(state))]
async fn history(
    State(state): State<AppState>,
    Path((dataset, document_id)): Path<(String, String)>,
    Query(q): Query<HistoryQuery>,
) -> ApiResult<Json<Value>> {
    let project = state.config().bootstrap_project.clone();
    let dataset_id = repo::resolve_dataset(state.pool(), &project, &dataset)
        .await
        .map_err(map_repo_err)?;

    let limit = q.limit.unwrap_or(100).clamp(1, 1000);

    // Join transactions → transaction_documents so we get the rev pair per tx.
    let rows = sqlx::query(
        r#"
        SELECT
            t.transaction_id,
            t.timestamp,
            t.author,
            t.mutations,
            td.previous_rev,
            td.result_rev
        FROM transactions t
        JOIN transaction_documents td
          ON td.transaction_id = t.id
        WHERE t.dataset_id = $1
          AND td.document_id = $2
        ORDER BY t.timestamp DESC
        LIMIT $3
        "#,
    )
    .bind(dataset_id)
    .bind(&document_id)
    .bind(limit)
    .fetch_all(state.pool())
    .await
    .map_err(ApiError::Database)?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let tx_id: String = row.try_get("transaction_id").map_err(ApiError::Database)?;
        let ts: sqlx::types::time::OffsetDateTime =
            row.try_get("timestamp").map_err(ApiError::Database)?;
        let author: Option<String> = row.try_get("author").ok();
        let mutations: Value = row.try_get("mutations").map_err(ApiError::Database)?;
        let previous_rev: Option<String> = row.try_get("previous_rev").ok();
        let result_rev: Option<String> = row.try_get("result_rev").ok();

        out.push(json!({
            "id": tx_id,
            "timestamp": odt_to_iso(ts),
            "author": author,
            "documentIDs": [document_id.clone()],
            "mutations": mutations,
            "resultRev": result_rev,
            "previousRev": previous_rev,
        }));
    }

    Ok(Json(json!({ "transactions": out })))
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
