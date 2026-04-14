//! Document repository layer.
//!
//! Thin, async functions over `sqlx::PgPool` (or a transaction connection)
//! that implement CRUD operations for Sanity documents stored in the
//! Content Lake's Postgres tables.
//!
//! All functions accept `impl sqlx::Executor<'_, Database = Postgres>` so
//! they compose naturally with transactions.

use chrono::{DateTime, TimeZone, Utc};
use serde_json::{Map, Value};
use sqlx::types::time::OffsetDateTime;
use sqlx::{Executor, Postgres, Row};
use thiserror::Error;
use uuid::Uuid;

use super::model::SanityDocument;

/// Convert a `time::OffsetDateTime` (what sqlx decodes for `TIMESTAMPTZ` when
/// the `time` feature is enabled) into a `chrono::DateTime<Utc>` so the rest
/// of the codebase can keep using chrono types.
fn offset_to_chrono(odt: OffsetDateTime) -> DateTime<Utc> {
    let unix_ns = odt.unix_timestamp_nanos();
    let mut secs = (unix_ns.div_euclid(1_000_000_000)) as i64;
    let mut nanos = (unix_ns.rem_euclid(1_000_000_000)) as u32;
    // `rem_euclid` is always non-negative so `nanos` is valid.
    if nanos >= 1_000_000_000 {
        secs += 1;
        nanos -= 1_000_000_000;
    }
    Utc.timestamp_opt(secs, nanos)
        .single()
        .unwrap_or_else(|| Utc.timestamp_opt(secs, 0).unwrap())
}

/// Errors returned by the repository layer.
#[derive(Debug, Error)]
pub enum RepoError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("already exists: {0}")]
    AlreadyExists(String),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

/// Convenient result alias for repo functions.
pub type RepoResult<T> = Result<T, RepoError>;

/// Generate a short UUID v7 hex revision string (16 hex chars).
fn new_revision() -> String {
    let s = Uuid::now_v7().as_simple().to_string();
    s[..16].to_string()
}

/// Merge the synthetic top-level Sanity fields (`_id`, `_type`, `_rev`,
/// `_createdAt`, `_updatedAt`) on top of a stored JSONB content map and
/// produce a [`SanityDocument`].
fn document_from_parts(
    document_id: String,
    doc_type: String,
    revision: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    content: Value,
) -> SanityDocument {
    let mut map: Map<String, Value> = match content {
        Value::Object(m) => m,
        _ => Map::new(),
    };

    // Strip any stale system fields that might have leaked into content.
    map.remove("_id");
    map.remove("_type");
    map.remove("_rev");
    map.remove("_createdAt");
    map.remove("_updatedAt");

    SanityDocument {
        _id: document_id,
        _type: doc_type,
        created_at,
        updated_at,
        _rev: revision,
        content: map,
    }
}

/// Idempotently ensure the given project+dataset pair exists and return
/// their UUIDs. Intended for server startup.
pub async fn bootstrap(
    pool: &sqlx::PgPool,
    project_name: &str,
    dataset_name: &str,
) -> RepoResult<(Uuid, Uuid)> {
    let mut tx = pool.begin().await?;

    let project_id: Uuid = sqlx::query_scalar(
        "INSERT INTO projects (name) VALUES ($1)
         ON CONFLICT (name) DO UPDATE SET name = EXCLUDED.name
         RETURNING id",
    )
    .bind(project_name)
    .fetch_one(&mut *tx)
    .await?;

    let dataset_id: Uuid = sqlx::query_scalar(
        "INSERT INTO datasets (project_id, name) VALUES ($1, $2)
         ON CONFLICT (project_id, name) DO UPDATE SET name = EXCLUDED.name
         RETURNING id",
    )
    .bind(project_id)
    .bind(dataset_name)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok((project_id, dataset_id))
}

/// Resolve a dataset UUID for a given `(project_name, dataset_name)` tuple.
pub async fn resolve_dataset<'e, E>(
    executor: E,
    project_name: &str,
    dataset_name: &str,
) -> RepoResult<Uuid>
where
    E: Executor<'e, Database = Postgres>,
{
    let row: Option<(Uuid,)> = sqlx::query_as(
        "SELECT d.id
         FROM datasets d
         INNER JOIN projects p ON p.id = d.project_id
         WHERE p.name = $1 AND d.name = $2",
    )
    .bind(project_name)
    .bind(dataset_name)
    .fetch_optional(executor)
    .await?;

    match row {
        Some((id,)) => Ok(id),
        None => Err(RepoError::NotFound(format!(
            "project '{project_name}' / dataset '{dataset_name}'"
        ))),
    }
}

/// Fetch a single document by its Sanity id. Returns `None` if missing or
/// soft-deleted.
pub async fn get_document<'e, E>(
    executor: E,
    dataset_id: Uuid,
    document_id: &str,
) -> RepoResult<Option<SanityDocument>>
where
    E: Executor<'e, Database = Postgres>,
{
    let row = sqlx::query(
        "SELECT document_id, doc_type, revision, content, created_at, updated_at
         FROM documents
         WHERE dataset_id = $1 AND document_id = $2 AND deleted = false",
    )
    .bind(dataset_id)
    .bind(document_id)
    .fetch_optional(executor)
    .await?;

    let Some(row) = row else {
        return Ok(None);
    };

    let doc = document_from_parts(
        row.try_get::<String, _>("document_id")?,
        row.try_get::<String, _>("doc_type")?,
        row.try_get::<String, _>("revision")?,
        offset_to_chrono(row.try_get::<OffsetDateTime, _>("created_at")?),
        offset_to_chrono(row.try_get::<OffsetDateTime, _>("updated_at")?),
        row.try_get::<Value, _>("content")?,
    );

    Ok(Some(doc))
}

/// Fetch many documents. Returns `(found, omitted)` where `omitted` is a
/// list of `(id, reason)` pairs for ids that were requested but could not
/// be returned (missing or soft-deleted; reason `"existence"`).
pub async fn get_documents<'e, E>(
    executor: E,
    dataset_id: Uuid,
    ids: &[String],
) -> RepoResult<(Vec<SanityDocument>, Vec<(String, String)>)>
where
    E: Executor<'e, Database = Postgres>,
{
    if ids.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }

    let rows = sqlx::query(
        "SELECT document_id, doc_type, revision, content, created_at, updated_at
         FROM documents
         WHERE dataset_id = $1
           AND document_id = ANY($2)
           AND deleted = false",
    )
    .bind(dataset_id)
    .bind(ids)
    .fetch_all(executor)
    .await?;

    let mut found: Vec<SanityDocument> = Vec::with_capacity(rows.len());
    let mut seen: std::collections::HashSet<String> =
        std::collections::HashSet::with_capacity(rows.len());

    for row in rows {
        let id: String = row.try_get("document_id")?;
        seen.insert(id.clone());
        let doc = document_from_parts(
            id,
            row.try_get::<String, _>("doc_type")?,
            row.try_get::<String, _>("revision")?,
            offset_to_chrono(row.try_get::<OffsetDateTime, _>("created_at")?),
            offset_to_chrono(row.try_get::<OffsetDateTime, _>("updated_at")?),
            row.try_get::<Value, _>("content")?,
        );
        found.push(doc);
    }

    let omitted: Vec<(String, String)> = ids
        .iter()
        .filter(|id| !seen.contains(id.as_str()))
        .map(|id| (id.clone(), "existence".to_string()))
        .collect();

    Ok((found, omitted))
}

/// Insert-or-replace (upsert) a document. Always bumps `_rev` and
/// `updated_at`. Use this for `createOrReplace` semantics.
///
/// If `revision` is `None`, a new revision is generated; otherwise the
/// caller-supplied value is used verbatim (useful for replaying log
/// events).
pub async fn upsert_document<'e, E>(
    executor: E,
    dataset_id: Uuid,
    id: &str,
    doc_type: &str,
    revision: Option<&str>,
    content: &Value,
) -> RepoResult<SanityDocument>
where
    E: Executor<'e, Database = Postgres>,
{
    let rev = revision.map(str::to_string).unwrap_or_else(new_revision);

    let row = sqlx::query(
        "INSERT INTO documents (dataset_id, document_id, doc_type, revision, content, updated_at, deleted)
         VALUES ($1, $2, $3, $4, $5, now(), false)
         ON CONFLICT (dataset_id, document_id) DO UPDATE
           SET doc_type = EXCLUDED.doc_type,
               revision = EXCLUDED.revision,
               content = EXCLUDED.content,
               updated_at = now(),
               deleted = false
         RETURNING document_id, doc_type, revision, content, created_at, updated_at",
    )
    .bind(dataset_id)
    .bind(id)
    .bind(doc_type)
    .bind(&rev)
    .bind(content)
    .fetch_one(executor)
    .await?;

    Ok(document_from_parts(
        row.try_get::<String, _>("document_id")?,
        row.try_get::<String, _>("doc_type")?,
        row.try_get::<String, _>("revision")?,
        offset_to_chrono(row.try_get::<OffsetDateTime, _>("created_at")?),
        offset_to_chrono(row.try_get::<OffsetDateTime, _>("updated_at")?),
        row.try_get::<Value, _>("content")?,
    ))
}

/// Strict insert; fails with [`RepoError::AlreadyExists`] if the id is
/// already present (even when soft-deleted). Use this for `create`
/// semantics.
pub async fn insert_document<'e, E>(
    executor: E,
    dataset_id: Uuid,
    id: &str,
    doc_type: &str,
    revision: Option<&str>,
    content: &Value,
) -> RepoResult<SanityDocument>
where
    E: Executor<'e, Database = Postgres>,
{
    let rev = revision.map(str::to_string).unwrap_or_else(new_revision);

    let result = sqlx::query(
        "INSERT INTO documents (dataset_id, document_id, doc_type, revision, content)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING document_id, doc_type, revision, content, created_at, updated_at",
    )
    .bind(dataset_id)
    .bind(id)
    .bind(doc_type)
    .bind(&rev)
    .bind(content)
    .fetch_one(executor)
    .await;

    match result {
        Ok(row) => Ok(document_from_parts(
            row.try_get::<String, _>("document_id")?,
            row.try_get::<String, _>("doc_type")?,
            row.try_get::<String, _>("revision")?,
            offset_to_chrono(row.try_get::<OffsetDateTime, _>("created_at")?),
            offset_to_chrono(row.try_get::<OffsetDateTime, _>("updated_at")?),
            row.try_get::<Value, _>("content")?,
        )),
        Err(sqlx::Error::Database(db_err)) if db_err.is_unique_violation() => {
            Err(RepoError::AlreadyExists(id.to_string()))
        }
        Err(e) => Err(RepoError::Database(e)),
    }
}

/// Soft-delete a document. Returns `true` when a row was affected.
pub async fn delete_document<'e, E>(
    executor: E,
    dataset_id: Uuid,
    id: &str,
) -> RepoResult<bool>
where
    E: Executor<'e, Database = Postgres>,
{
    let result = sqlx::query(
        "UPDATE documents
         SET deleted = true, updated_at = now()
         WHERE dataset_id = $1 AND document_id = $2 AND deleted = false",
    )
    .bind(dataset_id)
    .bind(id)
    .execute(executor)
    .await?;

    Ok(result.rows_affected() > 0)
}

/// Fetch only the current revision of a document, for optimistic
/// concurrency checks (`ifRevisionID`). Returns `None` when the document
/// is missing or soft-deleted.
pub async fn get_revision<'e, E>(
    executor: E,
    dataset_id: Uuid,
    id: &str,
) -> RepoResult<Option<String>>
where
    E: Executor<'e, Database = Postgres>,
{
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT revision FROM documents
         WHERE dataset_id = $1 AND document_id = $2 AND deleted = false",
    )
    .bind(dataset_id)
    .bind(id)
    .fetch_optional(executor)
    .await?;

    Ok(row.map(|(r,)| r))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn new_revision_is_16_hex_chars() {
        let rev = new_revision();
        assert_eq!(rev.len(), 16);
        assert!(rev.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn document_from_parts_merges_system_fields() {
        let now = Utc::now();
        let content = json!({ "title": "Hello", "body": "World" });
        let doc = document_from_parts(
            "drafts.abc".to_string(),
            "article".to_string(),
            "rev1".to_string(),
            now,
            now,
            content,
        );

        assert_eq!(doc._id, "drafts.abc");
        assert_eq!(doc._type, "article");
        assert_eq!(doc._rev, "rev1");
        assert_eq!(
            doc.content.get("title").and_then(|v| v.as_str()),
            Some("Hello")
        );
        // System fields should not live inside the content map.
        assert!(doc.content.get("_id").is_none());
        assert!(doc.content.get("_rev").is_none());

        // But they should serialize at the top level via #[serde(flatten)].
        let v = serde_json::to_value(&doc).unwrap();
        assert_eq!(v.get("_id").and_then(|v| v.as_str()), Some("drafts.abc"));
        assert_eq!(v.get("title").and_then(|v| v.as_str()), Some("Hello"));
    }

    #[test]
    fn document_from_parts_strips_stale_system_fields_in_content() {
        let now = Utc::now();
        let content = json!({
            "_id": "stale",
            "_rev": "stale",
            "title": "Hello",
        });
        let doc = document_from_parts(
            "real-id".to_string(),
            "article".to_string(),
            "real-rev".to_string(),
            now,
            now,
            content,
        );
        assert_eq!(doc._id, "real-id");
        assert_eq!(doc._rev, "real-rev");
        assert!(doc.content.get("_id").is_none());
        assert!(doc.content.get("_rev").is_none());
    }

    #[test]
    fn document_from_parts_handles_non_object_content() {
        let now = Utc::now();
        let doc = document_from_parts(
            "id".to_string(),
            "t".to_string(),
            "r".to_string(),
            now,
            now,
            Value::Null,
        );
        assert!(doc.content.is_empty());
    }

    // DB-backed integration tests would live here and be gated behind
    // a `DATABASE_URL`. They are intentionally stubbed out so the test
    // module exists as a known home for them.
    #[tokio::test]
    #[ignore = "requires a running Postgres; enable locally with DATABASE_URL set"]
    async fn bootstrap_is_idempotent() {
        // Placeholder integration test; wire up in a follow-up once
        // the test harness spins up ephemeral Postgres.
    }
}
