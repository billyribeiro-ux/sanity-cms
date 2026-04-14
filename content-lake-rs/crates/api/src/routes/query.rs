//! `GET/POST /v1/data/query/{dataset}` — execute a GROQ query.
//!
//! The query string is parsed by `content-lake-groq`, compiled to SQL via
//! `sql_gen::plan`, executed against Postgres, then reshaped in Rust for
//! projection / unpushed ordering / single-object return.

use std::collections::HashMap;
use std::time::Instant;

use axum::{
    extract::{Path, Query, State},
    routing::get,
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use sqlx::types::Uuid;
use sqlx::Row;

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

        let mut obj = content.as_object().cloned().unwrap_or_else(Map::new);
        obj.insert("_id".into(), Value::String(document_id));
        obj.insert("_type".into(), Value::String(doc_type));
        obj.insert("_rev".into(), Value::String(revision));
        obj.insert("_createdAt".into(), Value::String(odt_to_iso(created_at)));
        obj.insert("_updatedAt".into(), Value::String(odt_to_iso(updated_at)));
        results.push(Value::Object(obj));
    }

    // In-memory sort for ordering not pushed into SQL.
    sql_gen::sort_in_memory(&plan.order_by, &mut results);

    // count(*[...]) short-circuits: just return the integer.
    if plan.count_only {
        let ms = started.elapsed().as_millis() as u64;
        return Ok(Json(json!({
            "ms": ms,
            "query": query,
            "result": results.len() as u64,
        })));
    }

    // Apply projection (placeholders for Deref are inserted as _ref ids).
    let mut projected: Vec<Value> = if let Some(spec) = &plan.projection {
        results
            .iter()
            .map(|d| sql_gen::apply_projection(spec, d))
            .collect()
    } else {
        results
    };

    // Second pass: resolve dereferenced fields.
    if let Some(spec) = &plan.projection {
        resolve_derefs(&state, dataset_id, spec, &mut projected).await?;
    }

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

/// Second-pass resolution for `->` projections. Collects every unique `_ref`
/// id across all rows, fetches them in a single SQL round-trip, and patches
/// each projected doc with the resolved target.
async fn resolve_derefs(
    state: &AppState,
    dataset_id: sqlx::types::Uuid,
    spec: &sql_gen::ProjectionSpec,
    projected: &mut [Value],
) -> ApiResult<()> {
    use std::collections::{HashMap, HashSet};

    // Collect every id needed.
    let mut needed: HashSet<String> = HashSet::new();
    for doc in projected.iter() {
        for (_, id) in sql_gen::extract_deref_requests(spec, doc) {
            needed.insert(id);
        }
    }
    if needed.is_empty() {
        return Ok(());
    }

    // Fetch them all in one shot via the repo.
    let ids: Vec<String> = needed.into_iter().collect();
    let (found, _omitted) = repo::get_documents(state.pool(), dataset_id, &ids)
        .await
        .map_err(map_repo_err)?;

    // Materialize wire docs keyed by id.
    let mut by_id: HashMap<String, Value> = HashMap::with_capacity(found.len());
    for d in found {
        let id = d._id.clone();
        let mut obj = d.content;
        obj.insert("_id".into(), Value::String(id.clone()));
        obj.insert("_type".into(), Value::String(d._type));
        obj.insert("_rev".into(), Value::String(d._rev));
        obj.insert(
            "_createdAt".into(),
            Value::String(odt_to_iso_dt(d.created_at)),
        );
        obj.insert(
            "_updatedAt".into(),
            Value::String(odt_to_iso_dt(d.updated_at)),
        );
        by_id.insert(id, Value::Object(obj));
    }

    for doc in projected.iter_mut() {
        sql_gen::apply_deref_resolutions(spec, doc, &by_id);
    }
    Ok(())
}

fn odt_to_iso_dt(dt: DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
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
