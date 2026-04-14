//! `GET/POST /v1/data/query/{dataset}` — execute a GROQ query.
//!
//! The query string is parsed by `content-lake-groq`, compiled to SQL via
//! `sql_gen::plan`, executed against Postgres, then reshaped in Rust for
//! projection / unpushed ordering / single-object return.

use std::collections::HashMap;
use std::time::Instant;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::get,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use sqlx::Row;
use sqlx::types::Uuid;

use crate::{
    error::{ApiError, ApiResult},
    routes::doc::map_repo_err,
    state::AppState,
};

use content_lake_core::document::repo;
use content_lake_groq::{parser, sql_gen};

pub fn routes() -> Router<AppState> {
    Router::new().route("/v1/data/query/{dataset}", get(query_get).post(query_post))
}

#[derive(Debug, Deserialize)]
struct QueryPostBody {
    query: String,
    #[serde(default)]
    params: Option<Value>,
}

async fn query_get(
    State(state): State<AppState>,
    Path(dataset): Path<String>,
    Query(qs): Query<HashMap<String, String>>,
) -> ApiResult<Json<Value>> {
    let query = qs
        .get("query")
        .cloned()
        .ok_or_else(|| ApiError::BadRequest("missing ?query=".into()))?;
    let mut params = Map::new();
    for (k, v) in qs {
        if let Some(name) = k.strip_prefix('$') {
            // Try to parse as JSON first; fall back to string.
            let val: Value = serde_json::from_str(&v).unwrap_or(Value::String(v));
            params.insert(name.to_string(), val);
        }
    }
    run_query(state, dataset, query, params).await
}

async fn query_post(
    State(state): State<AppState>,
    Path(dataset): Path<String>,
    Json(body): Json<QueryPostBody>,
) -> ApiResult<Json<Value>> {
    let params = body
        .params
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    run_query(state, dataset, body.query, params).await
}

async fn run_query(
    state: AppState,
    dataset: String,
    query: String,
    params: Map<String, Value>,
) -> ApiResult<Json<Value>> {
    let started = Instant::now();

    let ast = parser::parse(&query)
        .map_err(|e| ApiError::BadRequest(format!("GROQ parse error: {e}")))?;
    let plan = sql_gen::plan(&ast, &params).map_err(map_plan_err)?;

    let project = state.config().bootstrap_project.clone();
    let dataset_id = repo::resolve_dataset(state.pool(), &project, &dataset)
        .await
        .map_err(map_repo_err)?;

    // Build a query with our bindings. $1 is dataset_id, $2.. come from plan.
    let mut q = sqlx::query(&plan.sql).bind(dataset_id);
    for v in &plan.bindings {
        q = bind_json(q, v);
    }

    let rows = q
        .fetch_all(state.pool())
        .await
        .map_err(ApiError::Database)?;

    // Shape each row into the wire document, then apply projection + ordering.
    let mut results: Vec<Value> = Vec::with_capacity(rows.len());
    for row in rows {
        let document_id: String = row.try_get("document_id").map_err(ApiError::Database)?;
        let doc_type: String = row.try_get("doc_type").map_err(ApiError::Database)?;
        let revision: String = row.try_get("revision").map_err(ApiError::Database)?;
        let content: Value = row.try_get("content").map_err(ApiError::Database)?;
        let created_at: sqlx::types::time::OffsetDateTime =
            row.try_get("created_at").map_err(ApiError::Database)?;
        let updated_at: sqlx::types::time::OffsetDateTime =
            row.try_get("updated_at").map_err(ApiError::Database)?;

        let mut obj = content
            .as_object()
            .cloned()
            .unwrap_or_else(Map::new);
        obj.insert("_id".into(), Value::String(document_id));
        obj.insert("_type".into(), Value::String(doc_type));
        obj.insert("_rev".into(), Value::String(revision));
        obj.insert("_createdAt".into(), Value::String(odt_to_iso(created_at)));
        obj.insert("_updatedAt".into(), Value::String(odt_to_iso(updated_at)));
        results.push(Value::Object(obj));
    }

    // In-memory sort for ordering not pushed into SQL.
    sql_gen::sort_in_memory(&plan.order_by, &mut results);

    // Apply projection.
    let mut projected: Vec<Value> = if let Some(spec) = &plan.projection {
        results.iter().map(|d| sql_gen::apply_projection(spec, d)).collect()
    } else {
        results
    };

    // If SQL didn't push a slice (e.g. negative indices) we could do it here.
    // For MVP the planner only produces positive slices which are pushed.

    let result = if plan.single {
        projected.into_iter().next().unwrap_or(Value::Null)
    } else {
        Value::Array(std::mem::take(&mut projected))
    };

    let ms = started.elapsed().as_millis() as u64;
    Ok(Json(json!({
        "ms": ms,
        "query": query,
        "result": result,
    })))
}

fn bind_json<'q>(
    q: sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>,
    v: &'q Value,
) -> sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments> {
    // Scalars: bind as native types so SQL comparisons work (e.g. doc_type = 'page').
    match v {
        Value::String(s) => q.bind(s.as_str()),
        Value::Bool(b) => q.bind(*b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                q.bind(i)
            } else if let Some(f) = n.as_f64() {
                q.bind(f)
            } else {
                q.bind(Value::Null)
            }
        }
        Value::Null => q.bind(Option::<String>::None),
        // Arrays/objects fall through as JSON text — limited support.
        other => q.bind(other.to_string()),
    }
}

fn map_plan_err(err: sql_gen::PlanError) -> ApiError {
    match err {
        sql_gen::PlanError::Unsupported(m) => {
            ApiError::BadRequest(format!("unsupported GROQ: {m}"))
        }
        sql_gen::PlanError::InvalidPath(m) => ApiError::BadRequest(format!("invalid path: {m}")),
        sql_gen::PlanError::ParamMissing(name) => {
            ApiError::BadRequest(format!("missing parameter: ${name}"))
        }
    }
}

#[allow(dead_code)]
fn odt_to_iso(odt: sqlx::types::time::OffsetDateTime) -> String {
    // Use the same chrono path as the rest of the codebase.
    let unix_ns = odt.unix_timestamp_nanos();
    let secs = (unix_ns.div_euclid(1_000_000_000)) as i64;
    let nanos = (unix_ns.rem_euclid(1_000_000_000)) as u32;
    let dt: DateTime<Utc> = chrono::TimeZone::timestamp_opt(&Utc, secs, nanos)
        .single()
        .unwrap_or_else(Utc::now);
    dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

// Reassure the compiler Uuid is used (future: for dataset scoping extensions).
#[allow(dead_code)]
fn _uuid_used(u: Uuid) -> Uuid {
    u
}
