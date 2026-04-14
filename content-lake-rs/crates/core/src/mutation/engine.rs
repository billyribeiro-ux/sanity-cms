//! Mutation engine — pure functions that apply a mutation to a document.
//!
//! This module is intentionally IO-free: it takes a `current` document state,
//! a `Mutation`, and returns either a new document state or a typed error.
//! The repository layer is responsible for actually reading/writing Postgres.
//!
//! Patch semantics follow Sanity's protocol:
//!
//! - `set`: assign values at paths (create intermediate objects as needed).
//! - `setIfMissing`: only set when the target is missing or null.
//! - `unset`: remove fields or array elements at the given paths.
//! - `inc` / `dec`: atomic numeric increment/decrement; errors on non-numbers.
//! - `insert`: `{before|after|replace: "path[idx]", items: [...]}`.
//! - `diffMatchPatch`: currently stubbed (returns [`MutationError::Unsupported`]).
//! - `ifRevisionID`: optimistic-concurrency guard checked before any op.

use chrono::{DateTime, Utc};
use serde_json::{Map, Value};
use thiserror::Error;

use crate::document::model::SanityDocument;
use crate::mutation::path as jp;
use crate::mutation::types::{
    CreateIfNotExistsMutation, CreateMutation, CreateOrReplaceMutation, DeleteMutation,
    DeleteTarget, InsertOperation, Mutation, PatchMutation, PatchOperations,
};

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MutationError {
    #[error("document already exists: {0}")]
    AlreadyExists(String),
    #[error("document not found: {0}")]
    NotFound(String),
    #[error("revision mismatch: expected {expected}, found {actual}")]
    RevisionMismatch { expected: String, actual: String },
    #[error("invalid path: {0}")]
    InvalidPath(String),
    #[error("invalid operation: {0}")]
    InvalidOperation(String),
    #[error("unsupported operation: {0}")]
    Unsupported(String),
}

/// The outcome of applying a single mutation.
#[derive(Debug, Clone)]
pub enum MutationResult {
    /// Created a new document.
    Created(SanityDocument),
    /// Updated an existing document (from `createOrReplace` or `patch`).
    Updated(SanityDocument),
    /// Deleted the document with the given id.
    Deleted(String),
    /// `createIfNotExists` found an existing document — nothing changed.
    NoOp,
}

/// Apply a mutation to the given document state.
///
/// `current` is the document state **before** the mutation, or `None` if the
/// target document does not exist. `new_revision` is the revision string the
/// engine will stamp onto freshly written documents (callers generate this
/// ahead of time, typically from `Uuid::now_v7`).
pub fn apply_mutation(
    current: Option<&SanityDocument>,
    mutation: &Mutation,
    new_revision: &str,
    now: DateTime<Utc>,
) -> Result<MutationResult, MutationError> {
    match mutation {
        Mutation::Create(m) => apply_create(current, m, new_revision, now),
        Mutation::CreateOrReplace(m) => apply_create_or_replace(current, m, new_revision, now),
        Mutation::CreateIfNotExists(m) => apply_create_if_not_exists(current, m, new_revision, now),
        Mutation::Delete(m) => apply_delete(current, m),
        Mutation::Patch(m) => apply_patch(current, m, new_revision, now),
    }
}

// -------------- Create family --------------

fn apply_create(
    current: Option<&SanityDocument>,
    m: &CreateMutation,
    rev: &str,
    now: DateTime<Utc>,
) -> Result<MutationResult, MutationError> {
    let doc = build_document_from_payload(&m.document, rev, now, /* is_replace */ false)?;
    if let Some(existing) = current {
        if existing._id == doc._id {
            return Err(MutationError::AlreadyExists(doc._id));
        }
    }
    Ok(MutationResult::Created(doc))
}

fn apply_create_or_replace(
    current: Option<&SanityDocument>,
    m: &CreateOrReplaceMutation,
    rev: &str,
    now: DateTime<Utc>,
) -> Result<MutationResult, MutationError> {
    let is_replace = current.is_some();
    let mut doc = build_document_from_payload(&m.document, rev, now, is_replace)?;
    if let Some(existing) = current {
        // Preserve original _createdAt when replacing.
        doc.created_at = existing.created_at;
    }
    if is_replace {
        Ok(MutationResult::Updated(doc))
    } else {
        Ok(MutationResult::Created(doc))
    }
}

fn apply_create_if_not_exists(
    current: Option<&SanityDocument>,
    m: &CreateIfNotExistsMutation,
    rev: &str,
    now: DateTime<Utc>,
) -> Result<MutationResult, MutationError> {
    if current.is_some() {
        return Ok(MutationResult::NoOp);
    }
    let doc = build_document_from_payload(&m.document, rev, now, false)?;
    Ok(MutationResult::Created(doc))
}

// -------------- Delete --------------

fn apply_delete(
    current: Option<&SanityDocument>,
    m: &DeleteMutation,
) -> Result<MutationResult, MutationError> {
    match &m.target {
        DeleteTarget::ById { id } => {
            validate_document_id(id)?;
            match current {
                Some(_) => Ok(MutationResult::Deleted(id.clone())),
                None => Err(MutationError::NotFound(id.clone())),
            }
        }
        DeleteTarget::ByQuery { .. } => Err(MutationError::Unsupported(
            "delete-by-query requires GROQ execution; not yet implemented in engine".into(),
        )),
    }
}

// -------------- Patch --------------

fn apply_patch(
    current: Option<&SanityDocument>,
    m: &PatchMutation,
    rev: &str,
    now: DateTime<Utc>,
) -> Result<MutationResult, MutationError> {
    let existing = current.ok_or_else(|| MutationError::NotFound(m.id.clone()))?;

    if let Some(expected) = &m.if_revision_id {
        if expected != &existing._rev {
            return Err(MutationError::RevisionMismatch {
                expected: expected.clone(),
                actual: existing._rev.clone(),
            });
        }
    }

    // Work on a JSON clone so we can use path helpers, then rebuild the document.
    let mut json = document_to_json(existing);

    apply_patch_ops(&mut json, &m.operations)?;

    // Re-stamp _rev and _updatedAt.
    if let Value::Object(ref mut map) = json {
        map.insert("_rev".into(), Value::String(rev.to_string()));
        map.insert("_updatedAt".into(), Value::String(iso(now)));
        // _createdAt preserved from the existing doc.
        map.insert("_createdAt".into(), Value::String(iso(existing.created_at)));
        map.insert("_id".into(), Value::String(existing._id.clone()));
        map.insert("_type".into(), Value::String(existing._type.clone()));
    }

    let updated = document_from_json(&json)?;
    Ok(MutationResult::Updated(updated))
}

fn apply_patch_ops(json: &mut Value, ops: &PatchOperations) -> Result<(), MutationError> {
    // 1. set
    if let Some(set) = &ops.set {
        let obj = set.as_object().ok_or_else(|| {
            MutationError::InvalidOperation("`set` must be an object of path→value".into())
        })?;
        for (path, value) in obj {
            jp::set_value(json, path, value.clone())
                .map_err(|e| MutationError::InvalidPath(format!("{path}: {e}")))?;
        }
    }

    // 2. setIfMissing
    if let Some(sim) = &ops.set_if_missing {
        let obj = sim.as_object().ok_or_else(|| {
            MutationError::InvalidOperation("`setIfMissing` must be an object".into())
        })?;
        for (path, value) in obj {
            let is_missing = matches!(jp::get_value(json, path), None | Some(Value::Null));
            if is_missing {
                jp::set_value(json, path, value.clone())
                    .map_err(|e| MutationError::InvalidPath(format!("{path}: {e}")))?;
            }
        }
    }

    // 3. unset
    if let Some(paths) = &ops.unset {
        for path in paths {
            jp::unset_value(json, path)
                .map_err(|e| MutationError::InvalidPath(format!("{path}: {e}")))?;
        }
    }

    // 4. inc / dec
    if let Some(inc) = &ops.inc {
        apply_inc_dec(json, inc, 1.0)?;
    }
    if let Some(dec) = &ops.dec {
        apply_inc_dec(json, dec, -1.0)?;
    }

    // 5. insert
    if let Some(ins) = &ops.insert {
        apply_insert(json, ins)?;
    }

    // 6. diffMatchPatch — not yet implemented
    if ops.diff_match_patch.is_some() {
        return Err(MutationError::Unsupported(
            "diffMatchPatch is not yet implemented".into(),
        ));
    }

    // 7. merge — shallow object merge at a path
    if let Some(merge) = &ops.merge {
        let obj = merge
            .as_object()
            .ok_or_else(|| MutationError::InvalidOperation("`merge` must be an object".into()))?;
        for (path, patch) in obj {
            let existing = jp::get_value(json, path).cloned().unwrap_or(Value::Null);
            let merged = deep_merge(existing, patch.clone());
            jp::set_value(json, path, merged)
                .map_err(|e| MutationError::InvalidPath(format!("{path}: {e}")))?;
        }
    }

    Ok(())
}

fn apply_inc_dec(json: &mut Value, op: &Value, sign: f64) -> Result<(), MutationError> {
    let obj = op
        .as_object()
        .ok_or_else(|| MutationError::InvalidOperation("`inc`/`dec` must be an object".into()))?;
    for (path, delta) in obj {
        let delta_num = delta
            .as_f64()
            .ok_or_else(|| MutationError::InvalidOperation(format!("`{path}` delta not numeric")))?;
        let current = jp::get_value(json, path).cloned().unwrap_or(Value::Null);
        let base = match &current {
            Value::Null => 0.0,
            Value::Number(n) => n
                .as_f64()
                .ok_or_else(|| MutationError::InvalidOperation(format!("`{path}` not f64-able")))?,
            _ => {
                return Err(MutationError::InvalidOperation(format!(
                    "`{path}` is not numeric (inc/dec target)"
                )));
            }
        };
        let new_val = base + sign * delta_num;
        let new_json = serde_json::Number::from_f64(new_val)
            .map(Value::Number)
            .unwrap_or(Value::Null);
        jp::set_value(json, path, new_json)
            .map_err(|e| MutationError::InvalidPath(format!("{path}: {e}")))?;
    }
    Ok(())
}

fn apply_insert(json: &mut Value, ins: &InsertOperation) -> Result<(), MutationError> {
    let (path, mode) = match (&ins.before, &ins.after, &ins.replace) {
        (Some(p), None, None) => (p, InsertMode::Before),
        (None, Some(p), None) => (p, InsertMode::After),
        (None, None, Some(p)) => (p, InsertMode::Replace),
        _ => {
            return Err(MutationError::InvalidOperation(
                "insert must specify exactly one of before|after|replace".into(),
            ));
        }
    };

    let (parent_segments, last_idx) = jp::split_array_path(path)
        .map_err(|e| MutationError::InvalidPath(format!("{path}: {e}")))?;

    let parent = jp::get_by_segments_mut(json, &parent_segments).ok_or_else(|| {
        MutationError::InvalidPath(format!("parent of {path} does not exist"))
    })?;

    let arr = parent.as_array_mut().ok_or_else(|| {
        MutationError::InvalidPath(format!("target of {path} is not an array"))
    })?;

    let resolved = resolve_index_for_insert(last_idx, arr.len(), mode)?;

    match mode {
        InsertMode::Before => {
            splice_in(arr, resolved, 0, &ins.items);
        }
        InsertMode::After => {
            splice_in(arr, resolved + 1, 0, &ins.items);
        }
        InsertMode::Replace => {
            splice_in(arr, resolved, 1, &ins.items);
        }
    }

    Ok(())
}

#[derive(Copy, Clone)]
enum InsertMode {
    Before,
    After,
    Replace,
}

fn resolve_index_for_insert(
    idx: i64,
    len: usize,
    mode: InsertMode,
) -> Result<usize, MutationError> {
    // Sanity allows -1 meaning "last"; for before/after we also permit the length-position.
    if idx >= 0 {
        let u = idx as usize;
        let upper = match mode {
            InsertMode::Before => len,
            InsertMode::After => len.saturating_sub(1).max(0),
            InsertMode::Replace => len,
        };
        if u > upper {
            return Err(MutationError::InvalidOperation(format!(
                "index {idx} out of bounds for array of length {len}"
            )));
        }
        Ok(u)
    } else {
        let abs = (-idx) as usize;
        if abs == 0 || abs > len {
            // Empty-array before/after [-1] is meaningful (insert at start)
            if len == 0 {
                return Ok(0);
            }
            return Err(MutationError::InvalidOperation(format!(
                "negative index {idx} out of bounds for array of length {len}"
            )));
        }
        Ok(len - abs)
    }
}

fn splice_in(arr: &mut Vec<Value>, at: usize, remove_count: usize, items: &[Value]) {
    let end = (at + remove_count).min(arr.len());
    let tail: Vec<Value> = arr.drain(at..end).collect();
    drop(tail);
    for (i, item) in items.iter().enumerate() {
        arr.insert(at + i, item.clone());
    }
}

// -------------- helpers --------------

fn build_document_from_payload(
    payload: &Value,
    rev: &str,
    now: DateTime<Utc>,
    is_replace: bool,
) -> Result<SanityDocument, MutationError> {
    let obj = payload.as_object().ok_or_else(|| {
        MutationError::InvalidOperation("document payload must be a JSON object".into())
    })?;

    let mut content = obj.clone();

    // _id: use payload or generate.
    let id = match content.remove("_id") {
        Some(Value::String(s)) => s,
        Some(_) => return Err(MutationError::InvalidOperation("_id must be a string".into())),
        None => uuid::Uuid::now_v7().as_simple().to_string(),
    };
    validate_document_id(&id)?;

    let doc_type = match content.remove("_type") {
        Some(Value::String(s)) => s,
        Some(_) => return Err(MutationError::InvalidOperation("_type must be a string".into())),
        None => {
            return Err(MutationError::InvalidOperation(
                "_type is required on create/replace".into(),
            ));
        }
    };

    // _createdAt: keep if already present (for create); for replace, caller overwrites.
    let created_at = match content.remove("_createdAt") {
        Some(Value::String(s)) => parse_iso(&s).unwrap_or(now),
        _ => now,
    };
    content.remove("_updatedAt");
    content.remove("_rev");

    Ok(SanityDocument {
        _id: id,
        _type: doc_type,
        created_at: if is_replace { now } else { created_at },
        updated_at: now,
        _rev: rev.to_string(),
        content,
    })
}

fn document_to_json(d: &SanityDocument) -> Value {
    let mut m = d.content.clone();
    m.insert("_id".into(), Value::String(d._id.clone()));
    m.insert("_type".into(), Value::String(d._type.clone()));
    m.insert("_rev".into(), Value::String(d._rev.clone()));
    m.insert("_createdAt".into(), Value::String(iso(d.created_at)));
    m.insert("_updatedAt".into(), Value::String(iso(d.updated_at)));
    Value::Object(m)
}

fn document_from_json(v: &Value) -> Result<SanityDocument, MutationError> {
    let mut m = v
        .as_object()
        .cloned()
        .ok_or_else(|| MutationError::InvalidOperation("expected object".into()))?;
    let id = take_string(&mut m, "_id")?;
    let doc_type = take_string(&mut m, "_type")?;
    let rev = take_string(&mut m, "_rev")?;
    let created_at = parse_iso(&take_string(&mut m, "_createdAt")?).unwrap_or_else(Utc::now);
    let updated_at = parse_iso(&take_string(&mut m, "_updatedAt")?).unwrap_or_else(Utc::now);
    Ok(SanityDocument {
        _id: id,
        _type: doc_type,
        created_at,
        updated_at,
        _rev: rev,
        content: m,
    })
}

fn take_string(m: &mut Map<String, Value>, key: &str) -> Result<String, MutationError> {
    match m.remove(key) {
        Some(Value::String(s)) => Ok(s),
        Some(_) => Err(MutationError::InvalidOperation(format!("{key} must be a string"))),
        None => Err(MutationError::InvalidOperation(format!("{key} missing"))),
    }
}

fn validate_document_id(id: &str) -> Result<(), MutationError> {
    if id.is_empty() {
        return Err(MutationError::InvalidOperation("_id cannot be empty".into()));
    }
    // Allow id, drafts.id, versions.release.id
    for ch in id.chars() {
        if !(ch.is_ascii_alphanumeric() || ch == '.' || ch == '-' || ch == '_') {
            return Err(MutationError::InvalidOperation(format!(
                "invalid _id character: {ch:?}"
            )));
        }
    }
    Ok(())
}

fn iso(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn parse_iso(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s).ok().map(|d| d.with_timezone(&Utc))
}

fn deep_merge(a: Value, b: Value) -> Value {
    match (a, b) {
        (Value::Object(mut am), Value::Object(bm)) => {
            for (k, v) in bm {
                let merged = deep_merge(am.remove(&k).unwrap_or(Value::Null), v);
                am.insert(k, merged);
            }
            Value::Object(am)
        }
        (_, other) => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn doc(id: &str, fields: Value) -> SanityDocument {
        let now = Utc::now();
        let mut content = fields.as_object().cloned().unwrap_or_default();
        content.remove("_id");
        content.remove("_type");
        SanityDocument {
            _id: id.to_string(),
            _type: "page".to_string(),
            created_at: now,
            updated_at: now,
            _rev: "rev1".to_string(),
            content,
        }
    }

    fn parse_mut(s: &str) -> Mutation {
        serde_json::from_str(s).unwrap()
    }

    #[test]
    fn create_happy_path() {
        let m = parse_mut(r#"{"create": {"_id": "a", "_type": "page", "title": "Hi"}}"#);
        let r = apply_mutation(None, &m, "rev2", Utc::now()).unwrap();
        match r {
            MutationResult::Created(d) => {
                assert_eq!(d._id, "a");
                assert_eq!(d._type, "page");
                assert_eq!(d._rev, "rev2");
                assert_eq!(d.content.get("title"), Some(&json!("Hi")));
            }
            _ => panic!("expected Created"),
        }
    }

    #[test]
    fn create_conflict() {
        let m = parse_mut(r#"{"create": {"_id": "a", "_type": "page"}}"#);
        let existing = doc("a", json!({}));
        let err = apply_mutation(Some(&existing), &m, "rev2", Utc::now()).unwrap_err();
        assert!(matches!(err, MutationError::AlreadyExists(_)));
    }

    #[test]
    fn create_if_not_exists_noop() {
        let m = parse_mut(r#"{"createIfNotExists": {"_id": "a", "_type": "page"}}"#);
        let existing = doc("a", json!({}));
        let r = apply_mutation(Some(&existing), &m, "rev2", Utc::now()).unwrap();
        assert!(matches!(r, MutationResult::NoOp));
    }

    #[test]
    fn create_or_replace_creates_when_missing() {
        let m = parse_mut(r#"{"createOrReplace": {"_id": "a", "_type": "page"}}"#);
        let r = apply_mutation(None, &m, "rev2", Utc::now()).unwrap();
        assert!(matches!(r, MutationResult::Created(_)));
    }

    #[test]
    fn create_or_replace_updates_when_present() {
        let m = parse_mut(r#"{"createOrReplace": {"_id": "a", "_type": "page", "title": "New"}}"#);
        let existing = doc("a", json!({"title": "Old"}));
        let r = apply_mutation(Some(&existing), &m, "rev2", Utc::now()).unwrap();
        match r {
            MutationResult::Updated(d) => {
                assert_eq!(d.content.get("title"), Some(&json!("New")));
            }
            _ => panic!("expected Updated"),
        }
    }

    #[test]
    fn delete_happy_path() {
        let m = parse_mut(r#"{"delete": {"id": "a"}}"#);
        let existing = doc("a", json!({}));
        let r = apply_mutation(Some(&existing), &m, "rev2", Utc::now()).unwrap();
        assert!(matches!(r, MutationResult::Deleted(ref id) if id == "a"));
    }

    #[test]
    fn delete_missing() {
        let m = parse_mut(r#"{"delete": {"id": "a"}}"#);
        let err = apply_mutation(None, &m, "rev2", Utc::now()).unwrap_err();
        assert!(matches!(err, MutationError::NotFound(_)));
    }

    #[test]
    fn patch_set_nested() {
        let m = parse_mut(r#"{"patch": {"id": "a", "set": {"meta.title": "Hi"}}}"#);
        let existing = doc("a", json!({"meta": {"author": "x"}}));
        let r = apply_mutation(Some(&existing), &m, "rev2", Utc::now()).unwrap();
        let d = match r { MutationResult::Updated(d) => d, _ => panic!() };
        assert_eq!(d.content.get("meta").unwrap().get("title"), Some(&json!("Hi")));
        assert_eq!(d.content.get("meta").unwrap().get("author"), Some(&json!("x")));
    }

    #[test]
    fn patch_unset() {
        let m = parse_mut(r#"{"patch": {"id": "a", "unset": ["title"]}}"#);
        let existing = doc("a", json!({"title": "X", "other": 1}));
        let r = apply_mutation(Some(&existing), &m, "rev2", Utc::now()).unwrap();
        let d = match r { MutationResult::Updated(d) => d, _ => panic!() };
        assert!(d.content.get("title").is_none());
    }

    #[test]
    fn patch_inc_on_number() {
        let m = parse_mut(r#"{"patch": {"id": "a", "inc": {"views": 5}}}"#);
        let existing = doc("a", json!({"views": 10}));
        let r = apply_mutation(Some(&existing), &m, "rev2", Utc::now()).unwrap();
        let d = match r { MutationResult::Updated(d) => d, _ => panic!() };
        assert_eq!(d.content.get("views"), Some(&json!(15.0)));
    }

    #[test]
    fn patch_inc_on_non_number_errors() {
        let m = parse_mut(r#"{"patch": {"id": "a", "inc": {"views": 5}}}"#);
        let existing = doc("a", json!({"views": "ten"}));
        let err = apply_mutation(Some(&existing), &m, "rev2", Utc::now()).unwrap_err();
        assert!(matches!(err, MutationError::InvalidOperation(_)));
    }

    #[test]
    fn patch_set_if_missing_skips_existing() {
        let m = parse_mut(r#"{"patch": {"id": "a", "setIfMissing": {"title": "New"}}}"#);
        let existing = doc("a", json!({"title": "Old"}));
        let r = apply_mutation(Some(&existing), &m, "rev2", Utc::now()).unwrap();
        let d = match r { MutationResult::Updated(d) => d, _ => panic!() };
        assert_eq!(d.content.get("title"), Some(&json!("Old")));
    }

    #[test]
    fn patch_if_revision_mismatch() {
        let m = parse_mut(
            r#"{"patch": {"id": "a", "ifRevisionID": "OLD", "set": {"x": 1}}}"#,
        );
        let existing = doc("a", json!({}));
        let err = apply_mutation(Some(&existing), &m, "rev2", Utc::now()).unwrap_err();
        assert!(matches!(err, MutationError::RevisionMismatch { .. }));
    }

    #[test]
    fn patch_insert_after() {
        let m = parse_mut(
            r#"{"patch": {"id": "a", "insert": {"after": "items[0]", "items": [99]}}}"#,
        );
        let existing = doc("a", json!({"items": [1, 2, 3]}));
        let r = apply_mutation(Some(&existing), &m, "rev2", Utc::now()).unwrap();
        let d = match r { MutationResult::Updated(d) => d, _ => panic!() };
        assert_eq!(d.content.get("items"), Some(&json!([1, 99, 2, 3])));
    }

    #[test]
    fn patch_insert_replace() {
        let m = parse_mut(
            r#"{"patch": {"id": "a", "insert": {"replace": "items[1]", "items": [99, 100]}}}"#,
        );
        let existing = doc("a", json!({"items": [1, 2, 3]}));
        let r = apply_mutation(Some(&existing), &m, "rev2", Utc::now()).unwrap();
        let d = match r { MutationResult::Updated(d) => d, _ => panic!() };
        assert_eq!(d.content.get("items"), Some(&json!([1, 99, 100, 3])));
    }

    #[test]
    fn patch_on_missing_doc_errors() {
        let m = parse_mut(r#"{"patch": {"id": "a", "set": {"x": 1}}}"#);
        let err = apply_mutation(None, &m, "rev2", Utc::now()).unwrap_err();
        assert!(matches!(err, MutationError::NotFound(_)));
    }

    #[test]
    fn patch_diff_match_patch_unsupported() {
        let m = parse_mut(
            r#"{"patch": {"id": "a", "diffMatchPatch": {"body": "@@..."}}}"#,
        );
        let existing = doc("a", json!({"body": "hi"}));
        let err = apply_mutation(Some(&existing), &m, "rev2", Utc::now()).unwrap_err();
        assert!(matches!(err, MutationError::Unsupported(_)));
    }

    #[test]
    fn invalid_id_rejected() {
        let m = parse_mut(r#"{"create": {"_id": "a b", "_type": "page"}}"#);
        let err = apply_mutation(None, &m, "rev2", Utc::now()).unwrap_err();
        assert!(matches!(err, MutationError::InvalidOperation(_)));
    }

    #[test]
    fn create_missing_type_rejected() {
        let m = parse_mut(r#"{"create": {"_id": "a"}}"#);
        let err = apply_mutation(None, &m, "rev2", Utc::now()).unwrap_err();
        assert!(matches!(err, MutationError::InvalidOperation(_)));
    }
}
