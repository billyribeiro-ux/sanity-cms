//! GROQ → SQL query planner.
//!
//! Scope: the subset of GROQ needed by Sanity Studio's structure tool and
//! reference pickers. We target queries shaped like:
//!
//! ```groq
//! *[_type == $type && slug.current == $slug]
//!   | order(_createdAt desc)
//!   [0..20]
//!   {title, slug, "body": content.body}
//! ```
//!
//! The generator walks the AST, extracts the filter into a WHERE clause
//! over `documents.content` (JSONB) + the `doc_type` column, and returns
//! a [`QueryPlan`] containing the SQL + ordered bindings. Projection,
//! ordering, and slicing are applied in Rust after the SQL fetch if they
//! can't be pushed down cleanly.

use serde_json::{Map, Value};
use thiserror::Error;

use crate::ast::Expr;

#[derive(Debug, Error)]
pub enum PlanError {
    #[error("unsupported GROQ feature: {0}")]
    Unsupported(String),
    #[error("invalid path: {0}")]
    InvalidPath(String),
    #[error("missing parameter: ${0}")]
    ParamMissing(String),
}

/// A compiled SQL plan plus post-fetch shaping hints.
#[derive(Debug, Clone)]
pub struct QueryPlan {
    /// Fully-formed SQL string with `$1`, `$2`, ... placeholders.
    /// Always scans `documents` with `dataset_id = $1 AND NOT deleted`.
    pub sql: String,
    /// Bindings in order: `$1` is always `dataset_id` (filled by the caller),
    /// `$2..` are the values produced here.
    pub bindings: Vec<Value>,
    /// Projection spec to apply in Rust after fetching. None = return docs as-is.
    pub projection: Option<ProjectionSpec>,
    /// Optional `[start..end)` slice applied after ordering.
    pub slice: Option<(i64, Option<i64>)>,
    /// Ordering keys applied in Rust if not pushed into SQL.
    pub order_by: Vec<OrderKey>,
    /// When true, return a single object (not an array) — used for `*[...][0]`.
    pub single: bool,
    /// When true, the executor should return `{"ms":…,"query":…,"result": N}`
    /// where N is the row count — used for `count(*[...])`.
    pub count_only: bool,
}

#[derive(Debug, Clone)]
pub struct ProjectionSpec {
    /// Each entry is (output_name, source).
    pub fields: Vec<(String, ProjectionSource)>,
}

#[derive(Debug, Clone)]
pub enum ProjectionSource {
    /// `title` — copy the `title` field from the document.
    Ident(String),
    /// `"alias": path.to.field` — copy a dotted path under an alias.
    Path(Vec<String>),
    /// `"alias": ref->` — follow the reference at `path`, return the target doc
    /// (or subfield if `sub_path` is non-empty: `ref->name` gives the name).
    Deref {
        path: Vec<String>,
        /// Optional projection of the target doc. None = return the whole target doc.
        sub_projection: Option<Box<ProjectionSpec>>,
    },
    /// Any expression we couldn't specialize — materialize as null.
    Unsupported,
}

#[derive(Debug, Clone)]
pub struct OrderKey {
    /// Dotted path into the document, e.g. `["_createdAt"]` or `["meta","views"]`.
    pub path: Vec<String>,
    pub descending: bool,
}

/// Compile a parsed GROQ expression into a [`QueryPlan`].
///
/// The caller must prepend the `dataset_id` binding as `$1` when executing
/// the resulting SQL.
pub fn plan(ast: &Expr, params: &Map<String, Value>) -> Result<QueryPlan, PlanError> {
    // Unwrap `count(x)` at the top level: plan `x` but mark the plan so the
    // executor returns a single number.
    let (root, count_only) = match ast {
        Expr::FuncCall(name, args) if name == "count" && args.len() == 1 => {
            (args[0].clone(), true)
        }
        other => (other.clone(), false),
    };

    // Normalize: most meaningful queries are pipelines. Single-stage things
    // like bare `*` are accepted too.
    let stages = match &root {
        Expr::Pipeline(stages) => stages.clone(),
        other => vec![other.clone()],
    };

    let mut filter: Option<Expr> = None;
    let mut projection: Option<ProjectionSpec> = None;
    let mut order_by: Vec<OrderKey> = Vec::new();
    let mut slice: Option<(i64, Option<i64>)> = None;
    let mut single = false;

    for stage in &stages {
        match stage {
            Expr::Everything => { /* base table scan is implicit */ }
            Expr::Filter(pred) => {
                filter = Some(*pred.clone());
            }
            Expr::Projection(fields) => {
                projection = Some(build_projection_spec(fields)?);
            }
            Expr::Order(expr, ascending) => {
                let path = extract_path(expr)?;
                order_by.push(OrderKey {
                    path,
                    descending: !ascending,
                });
            }
            Expr::Slice(_, start, end) => {
                if start == end {
                    // Parser emits `Slice(This, n, n)` for `[n]` single-pick.
                    single = true;
                    slice = Some((*start, Some(*start + 1)));
                } else {
                    slice = Some((*start, Some(*end)));
                }
            }
            other => {
                return Err(PlanError::Unsupported(format!(
                    "pipeline stage not supported: {other:?}"
                )));
            }
        }
    }

    // ---- Build SQL ----
    let mut bindings: Vec<Value> = Vec::new();
    let mut where_extra = String::new();

    if let Some(f) = &filter {
        let mut ctx = SqlCtx {
            bindings: &mut bindings,
            params,
        };
        let cond = emit_predicate(f, &mut ctx)?;
        where_extra = format!(" AND ({cond})");
    }

    let mut sql = String::from(
        "SELECT document_id, doc_type, revision, content, created_at, updated_at \
         FROM documents \
         WHERE dataset_id = $1 AND NOT deleted",
    );
    sql.push_str(&where_extra);

    // Push ordering to SQL for well-known columns. Everything else sorts in Rust.
    let pushable_order: Vec<String> = order_by
        .iter()
        .filter_map(|k| {
            column_for_order(&k.path).map(|c| {
                let dir = if k.descending { "DESC" } else { "ASC" };
                format!("{c} {dir}")
            })
        })
        .collect();
    if !pushable_order.is_empty() {
        sql.push_str(" ORDER BY ");
        sql.push_str(&pushable_order.join(", "));
    }

    if let Some((start, end)) = slice {
        if start >= 0 {
            sql.push_str(&format!(" OFFSET {start}"));
            if let Some(e) = end {
                let lim = (e - start).max(0);
                sql.push_str(&format!(" LIMIT {lim}"));
            }
        }
    }

    // Keep unpushed ordering keys for in-memory sort after fetch.
    let unpushed_order = if pushable_order.is_empty() {
        order_by
    } else {
        Vec::new()
    };

    Ok(QueryPlan {
        sql,
        bindings,
        projection,
        slice,
        order_by: unpushed_order,
        single,
        count_only,
    })
}

// ---- Predicate → SQL ----

struct SqlCtx<'a> {
    bindings: &'a mut Vec<Value>,
    params: &'a Map<String, Value>,
}

fn emit_predicate(expr: &Expr, ctx: &mut SqlCtx) -> Result<String, PlanError> {
    match expr {
        Expr::Eq(l, r) => bin_cmp(l, r, "=", ctx),
        Expr::Neq(l, r) => bin_cmp(l, r, "<>", ctx),
        Expr::Lt(l, r) => bin_cmp(l, r, "<", ctx),
        Expr::Lte(l, r) => bin_cmp(l, r, "<=", ctx),
        Expr::Gt(l, r) => bin_cmp(l, r, ">", ctx),
        Expr::Gte(l, r) => bin_cmp(l, r, ">=", ctx),
        Expr::And(l, r) => {
            let ls = emit_predicate(l, ctx)?;
            let rs = emit_predicate(r, ctx)?;
            Ok(format!("({ls}) AND ({rs})"))
        }
        Expr::Or(l, r) => {
            let ls = emit_predicate(l, ctx)?;
            let rs = emit_predicate(r, ctx)?;
            Ok(format!("({ls}) OR ({rs})"))
        }
        Expr::Not(inner) => {
            let s = emit_predicate(inner, ctx)?;
            Ok(format!("NOT ({s})"))
        }
        Expr::FuncCall(name, args) if name == "defined" && args.len() == 1 => {
            let sel = path_selector(&args[0])?;
            Ok(format!("{sel} IS NOT NULL"))
        }
        Expr::FuncCall(name, args) if name == "references" && args.len() == 1 => {
            // references("id") → true if any JSONB leaf contains {_ref: "id"}.
            // We use a jsonpath containment check on the full `content` column.
            let id_val = match &args[0] {
                Expr::StringLiteral(s) => Value::String(s.clone()),
                Expr::Param(p) => ctx
                    .params
                    .get(p)
                    .cloned()
                    .ok_or_else(|| PlanError::ParamMissing(p.clone()))?,
                other => {
                    return Err(PlanError::Unsupported(format!(
                        "references() needs a string or param, got {other:?}"
                    )));
                }
            };
            ctx.bindings.push(id_val);
            let idx = ctx.bindings.len() + 1;
            Ok(format!(
                "jsonb_path_exists(content, ('$.** ? (@._ref == \"' || ${idx} || '\")')::jsonpath)"
            ))
        }
        Expr::In(l, r) => emit_in(l, r, ctx),
        Expr::FuncCall(name, args) if (name == "match" || name.is_empty()) && args.len() == 2 => {
            emit_match(&args[0], &args[1], ctx)
        }
        // `string match pattern` — parser represents `x match "y*"` as FuncCall("match", [x, y]).
        // If we encounter it as a plain Expr::Eq-like infix we'd need a token, but we don't.
        other => Err(PlanError::Unsupported(format!(
            "filter expression not supported: {other:?}"
        ))),
    }
}

fn emit_in(l: &Expr, r: &Expr, ctx: &mut SqlCtx) -> Result<String, PlanError> {
    if !is_path_expr(l) {
        return Err(PlanError::Unsupported(format!(
            "left side of `in` must be a path, got {l:?}"
        )));
    }
    let sel = path_selector(l)?;

    match r {
        Expr::Array(items) => {
            let mut placeholders = Vec::with_capacity(items.len());
            for item in items {
                if !is_value_expr(item) {
                    return Err(PlanError::Unsupported(
                        "`in` array must contain only literal/param values".into(),
                    ));
                }
                placeholders.push(bind_value(item, ctx)?);
            }
            Ok(format!("{sel} IN ({})", placeholders.join(", ")))
        }
        Expr::Param(name) => {
            // `x in $ids` — the param value is an array. Bind each element.
            let arr = ctx
                .params
                .get(name)
                .cloned()
                .ok_or_else(|| PlanError::ParamMissing(name.clone()))?;
            let Value::Array(items) = arr else {
                return Err(PlanError::Unsupported(format!(
                    "param ${name} for `in` must be an array"
                )));
            };
            if items.is_empty() {
                return Ok("FALSE".to_string());
            }
            let mut placeholders = Vec::with_capacity(items.len());
            for v in items {
                ctx.bindings.push(v);
                placeholders.push(format!("${}", ctx.bindings.len() + 1));
            }
            Ok(format!("{sel} IN ({})", placeholders.join(", ")))
        }
        other => Err(PlanError::Unsupported(format!(
            "right side of `in` must be array or param, got {other:?}"
        ))),
    }
}

/// `path match "pattern"` — supports glob-style `*` and `?`. Compiled to SQL
/// `ILIKE` with the pattern chars translated (`*` → `%`, `?` → `_`).
fn emit_match(l: &Expr, r: &Expr, ctx: &mut SqlCtx) -> Result<String, PlanError> {
    if !is_path_expr(l) {
        return Err(PlanError::Unsupported(format!(
            "left side of `match` must be a path, got {l:?}"
        )));
    }
    let sel = path_selector(l)?;
    let pattern = match r {
        Expr::StringLiteral(s) => s.clone(),
        Expr::Param(name) => ctx
            .params
            .get(name)
            .cloned()
            .and_then(|v| v.as_str().map(String::from))
            .ok_or_else(|| PlanError::ParamMissing(name.clone()))?,
        other => {
            return Err(PlanError::Unsupported(format!(
                "`match` right side must be a string, got {other:?}"
            )));
        }
    };
    let ilike = glob_to_ilike(&pattern);
    ctx.bindings.push(Value::String(ilike));
    Ok(format!("{sel} ILIKE ${}", ctx.bindings.len() + 1))
}

fn glob_to_ilike(pattern: &str) -> String {
    // Escape SQL LIKE specials (% and _), then translate glob *, ? to LIKE %, _.
    let mut out = String::with_capacity(pattern.len() + 2);
    for c in pattern.chars() {
        match c {
            '*' => out.push('%'),
            '?' => out.push('_'),
            '%' => out.push_str("\\%"),
            '_' => out.push_str("\\_"),
            '\\' => out.push_str("\\\\"),
            other => out.push(other),
        }
    }
    out
}

fn bin_cmp(l: &Expr, r: &Expr, op: &str, ctx: &mut SqlCtx) -> Result<String, PlanError> {
    // Permit `path OP value` or `value OP path`.
    if is_value_expr(l) && is_path_expr(r) {
        let sel = path_selector(r)?;
        let binding = bind_value(l, ctx)?;
        return Ok(format!("{sel} {op} {binding}"));
    }
    if is_value_expr(r) && is_path_expr(l) {
        let sel = path_selector(l)?;
        let binding = bind_value(r, ctx)?;
        return Ok(format!("{sel} {op} {binding}"));
    }
    Err(PlanError::Unsupported(format!(
        "comparison must be `path OP value` or `value OP path`; got {l:?} {op} {r:?}"
    )))
}

fn is_value_expr(e: &Expr) -> bool {
    matches!(
        e,
        Expr::StringLiteral(_)
            | Expr::IntLiteral(_)
            | Expr::FloatLiteral(_)
            | Expr::BoolLiteral(_)
            | Expr::Null
            | Expr::Param(_)
    )
}

fn is_path_expr(e: &Expr) -> bool {
    matches!(e, Expr::Ident(_) | Expr::DotAccess(_, _) | Expr::This)
}

fn bind_value(e: &Expr, ctx: &mut SqlCtx) -> Result<String, PlanError> {
    let val = match e {
        Expr::StringLiteral(s) => Value::String(s.clone()),
        Expr::IntLiteral(n) => Value::from(*n),
        Expr::FloatLiteral(f) => Value::from(*f),
        Expr::BoolLiteral(b) => Value::Bool(*b),
        Expr::Null => Value::Null,
        Expr::Param(name) => ctx
            .params
            .get(name)
            .cloned()
            .ok_or_else(|| PlanError::ParamMissing(name.clone()))?,
        other => {
            return Err(PlanError::Unsupported(format!(
                "value expression not supported: {other:?}"
            )));
        }
    };
    ctx.bindings.push(val);
    // $1 is dataset_id; our first binding is $2.
    Ok(format!("${}", ctx.bindings.len() + 1))
}

/// Turn a path expression into a SQL selector against the `documents` row.
///
/// `_id`, `_type`, `_rev`, `_createdAt`, `_updatedAt` go to native columns;
/// everything else reads `content` (JSONB) as text.
fn path_selector(expr: &Expr) -> Result<String, PlanError> {
    let path = extract_path(expr)?;
    if path.len() == 1 {
        match path[0].as_str() {
            "_id" => return Ok("document_id".to_string()),
            "_type" => return Ok("doc_type".to_string()),
            "_rev" => return Ok("revision".to_string()),
            "_createdAt" => {
                return Ok(
                    "to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"')"
                        .to_string(),
                );
            }
            "_updatedAt" => {
                return Ok(
                    "to_char(updated_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"')"
                        .to_string(),
                );
            }
            _ => {}
        }
    }
    let escaped: Vec<String> = path.iter().map(|p| p.replace('\'', "''")).collect();
    Ok(format!("(content #>> '{{{}}}')", escaped.join(",")))
}

/// Extract a dotted path from an expression like `a.b.c` → `["a","b","c"]`.
pub fn extract_path(expr: &Expr) -> Result<Vec<String>, PlanError> {
    let mut parts: Vec<String> = Vec::new();
    collect_path(expr, &mut parts)?;
    if parts.is_empty() {
        return Err(PlanError::InvalidPath("empty path".into()));
    }
    Ok(parts)
}

fn collect_path(expr: &Expr, parts: &mut Vec<String>) -> Result<(), PlanError> {
    match expr {
        Expr::Ident(s) => {
            parts.push(s.clone());
            Ok(())
        }
        Expr::This => Ok(()),
        Expr::DotAccess(inner, name) => {
            collect_path(inner, parts)?;
            parts.push(name.clone());
            Ok(())
        }
        other => Err(PlanError::InvalidPath(format!("not a path: {other:?}"))),
    }
}

fn column_for_order(path: &[String]) -> Option<String> {
    if path.len() == 1 {
        match path[0].as_str() {
            "_id" => return Some("document_id".into()),
            "_type" => return Some("doc_type".into()),
            "_createdAt" => return Some("created_at".into()),
            "_updatedAt" => return Some("updated_at".into()),
            _ => {}
        }
    }
    None
}

// ---- Projection specialization ----

fn build_projection_spec(fields: &[(String, Expr)]) -> Result<ProjectionSpec, PlanError> {
    let mut out = Vec::with_capacity(fields.len());
    for (name, expr) in fields {
        let src = classify_projection(expr)?;
        out.push((name.clone(), src));
    }
    Ok(ProjectionSpec { fields: out })
}

fn classify_projection(expr: &Expr) -> Result<ProjectionSource, PlanError> {
    match expr {
        Expr::Ident(s) => Ok(ProjectionSource::Ident(s.clone())),
        Expr::DotAccess(..) | Expr::This => match extract_path(expr) {
            Ok(p) if !p.is_empty() => Ok(ProjectionSource::Path(p)),
            _ => Ok(ProjectionSource::Unsupported),
        },
        Expr::Deref(inner, field) => {
            // `a->field` → read the `_ref` at path `a`, resolve the doc, then
            // extract `field` from it. Represent as Deref with sub_projection
            // containing a single Ident field.
            let path = extract_path(inner)?;
            let sub = ProjectionSpec {
                fields: vec![(field.clone(), ProjectionSource::Ident(field.clone()))],
            };
            Ok(ProjectionSource::Deref {
                path,
                sub_projection: Some(Box::new(sub)),
            })
        }
        _ => Ok(ProjectionSource::Unsupported),
    }
}

/// Apply a [`ProjectionSpec`] to an already-shaped document (with system fields).
///
/// `Deref` sources are NOT resolved here — the caller must post-process by
/// inspecting [`extract_deref_requests`] and filling the resolved docs back
/// in via [`apply_deref_resolutions`]. This keeps the sync function IO-free.
pub fn apply_projection(spec: &ProjectionSpec, doc: &Value) -> Value {
    let mut out = Map::new();
    for (name, src) in &spec.fields {
        let v = match src {
            ProjectionSource::Ident(s) => doc.get(s).cloned().unwrap_or(Value::Null),
            ProjectionSource::Path(parts) => resolve_path(doc, parts),
            ProjectionSource::Deref { path, .. } => {
                // Leave a placeholder containing the _ref id so the executor
                // can resolve in a second pass. null if no ref present.
                let v = resolve_path(doc, path);
                v.get("_ref").cloned().unwrap_or(Value::Null)
            }
            ProjectionSource::Unsupported => Value::Null,
        };
        out.insert(name.clone(), v);
    }
    Value::Object(out)
}

/// Walk a [`ProjectionSpec`] and return the list of `(output_field_name, _ref_id)`
/// pairs the executor still needs to resolve. Input is the already-projected
/// document (from [`apply_projection`]).
pub fn extract_deref_requests(
    spec: &ProjectionSpec,
    projected_doc: &Value,
) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (name, src) in &spec.fields {
        if let ProjectionSource::Deref { .. } = src {
            if let Some(Value::String(id)) = projected_doc.get(name) {
                out.push((name.clone(), id.clone()));
            }
        }
    }
    out
}

/// Replace Deref placeholders in a projected document with the resolved target
/// (optionally further projected via `sub_projection`).
pub fn apply_deref_resolutions(
    spec: &ProjectionSpec,
    projected_doc: &mut Value,
    resolutions: &std::collections::HashMap<String, Value>,
) {
    let obj = match projected_doc.as_object_mut() {
        Some(o) => o,
        None => return,
    };
    for (name, src) in &spec.fields {
        if let ProjectionSource::Deref { sub_projection, .. } = src {
            let id_opt = obj.get(name).and_then(|v| v.as_str()).map(String::from);
            let Some(id) = id_opt else { continue };
            let target = resolutions.get(&id).cloned().unwrap_or(Value::Null);
            let resolved = match sub_projection {
                Some(sub) => apply_projection(sub, &target),
                None => target,
            };
            obj.insert(name.clone(), resolved);
        }
    }
}

fn resolve_path(root: &Value, parts: &[String]) -> Value {
    let mut cur = root;
    for p in parts {
        let Some(next) = cur.get(p) else {
            return Value::Null;
        };
        cur = next;
    }
    cur.clone()
}

/// In-memory sort for ordering keys we didn't push to SQL.
pub fn sort_in_memory(order_by: &[OrderKey], docs: &mut [Value]) {
    if order_by.is_empty() {
        return;
    }
    docs.sort_by(|a, b| {
        for key in order_by {
            let av = resolve_path(a, &key.path);
            let bv = resolve_path(b, &key.path);
            let ord = compare_json(&av, &bv);
            let final_ord = if key.descending { ord.reverse() } else { ord };
            if final_ord != std::cmp::Ordering::Equal {
                return final_ord;
            }
        }
        std::cmp::Ordering::Equal
    });
}

fn compare_json(a: &Value, b: &Value) -> std::cmp::Ordering {
    use std::cmp::Ordering::*;
    match (a, b) {
        (Value::Null, Value::Null) => Equal,
        (Value::Null, _) => Less,
        (_, Value::Null) => Greater,
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        (Value::Number(x), Value::Number(y)) => {
            let xf = x.as_f64().unwrap_or(0.0);
            let yf = y.as_f64().unwrap_or(0.0);
            xf.partial_cmp(&yf).unwrap_or(Equal)
        }
        (Value::String(x), Value::String(y)) => x.cmp(y),
        _ => Equal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;
    use serde_json::json;

    fn do_plan(q: &str, params: Value) -> QueryPlan {
        let ast = parse(q).expect("parse");
        let p_obj = params.as_object().cloned().unwrap_or_default();
        plan(&ast, &p_obj).expect("plan")
    }

    #[test]
    fn plain_everything() {
        let p = do_plan("*", json!({}));
        assert!(p.sql.contains("FROM documents"));
        assert!(p.bindings.is_empty());
    }

    #[test]
    fn filter_by_type() {
        let p = do_plan(r#"*[_type == "page"]"#, json!({}));
        assert!(p.sql.contains("doc_type = $2"));
        assert_eq!(p.bindings, vec![json!("page")]);
    }

    #[test]
    fn filter_reversed_sides() {
        let p = do_plan(r#"*["page" == _type]"#, json!({}));
        assert!(p.sql.contains("doc_type = $2"));
        assert_eq!(p.bindings, vec![json!("page")]);
    }

    #[test]
    fn filter_param_substitution() {
        let p = do_plan(r#"*[_type == $type]"#, json!({"type": "article"}));
        assert_eq!(p.bindings, vec![json!("article")]);
    }

    #[test]
    fn filter_nested_path() {
        let p = do_plan(r#"*[slug.current == "hello"]"#, json!({}));
        assert!(p.sql.contains("content #>> '{slug,current}'"));
    }

    #[test]
    fn filter_and() {
        let p = do_plan(
            r#"*[_type == "page" && slug.current == "hello"]"#,
            json!({}),
        );
        assert!(p.sql.contains("AND"));
        assert_eq!(p.bindings.len(), 2);
    }

    #[test]
    fn filter_or() {
        let p = do_plan(r#"*[_type == "page" || _type == "post"]"#, json!({}));
        assert!(p.sql.contains("OR"));
    }

    #[test]
    fn filter_not() {
        let p = do_plan(r#"*[!(_type == "draft")]"#, json!({}));
        assert!(p.sql.contains("NOT"));
    }

    #[test]
    fn defined_function() {
        let p = do_plan(r#"*[defined(slug.current)]"#, json!({}));
        assert!(p.sql.contains("IS NOT NULL"));
    }

    #[test]
    fn order_pushes_createdat_to_sql() {
        let p = do_plan(r#"*[_type == "page"] | order(_createdAt desc)"#, json!({}));
        assert!(p.sql.contains("ORDER BY created_at DESC"));
        assert!(p.order_by.is_empty());
    }

    #[test]
    fn order_nonstd_falls_back_to_memory() {
        let p = do_plan(r#"*[_type == "page"] | order(title asc)"#, json!({}));
        assert!(!p.sql.contains("ORDER BY"));
        assert_eq!(p.order_by.len(), 1);
        assert_eq!(p.order_by[0].path, vec!["title".to_string()]);
    }

    #[test]
    fn slice_pushes_limit_offset() {
        let p = do_plan(r#"*[_type == "page"][5..15]"#, json!({}));
        assert!(p.sql.contains("OFFSET 5"));
        assert!(p.sql.contains("LIMIT 10"));
    }

    #[test]
    fn missing_param_errors() {
        let ast = parse(r#"*[_type == $type]"#).unwrap();
        let err = plan(&ast, &Map::new()).unwrap_err();
        assert!(matches!(err, PlanError::ParamMissing(_)));
    }

    #[test]
    fn projection_spec_extracted() {
        let p = do_plan(r#"*[_type == "page"]{title, slug}"#, json!({}));
        let spec = p.projection.expect("projection");
        assert_eq!(spec.fields.len(), 2);
        match &spec.fields[0].1 {
            ProjectionSource::Ident(s) => assert_eq!(s, "title"),
            _ => panic!("expected Ident for title"),
        }
    }

    #[test]
    fn apply_projection_picks_fields() {
        let doc = json!({"_id": "a", "_type": "page", "title": "Hi", "slug": {"current": "hi"}});
        let spec = ProjectionSpec {
            fields: vec![
                ("title".into(), ProjectionSource::Ident("title".into())),
                ("slug".into(), ProjectionSource::Path(vec!["slug".into(), "current".into()])),
            ],
        };
        let out = apply_projection(&spec, &doc);
        assert_eq!(out, json!({"title": "Hi", "slug": "hi"}));
    }

    #[test]
    fn count_wrapper_sets_flag() {
        let p = do_plan(r#"count(*[_type == "page"])"#, json!({}));
        assert!(p.count_only);
        assert!(p.sql.contains("doc_type = $2"));
    }

    #[test]
    fn in_with_array_literal() {
        let p = do_plan(
            r#"*[_type in ["page", "post", "note"]]"#,
            json!({}),
        );
        assert!(p.sql.contains("IN ($2, $3, $4)"));
        assert_eq!(p.bindings.len(), 3);
    }

    #[test]
    fn in_with_param_array() {
        let p = do_plan(
            r#"*[_id in $ids]"#,
            json!({"ids": ["a", "b"]}),
        );
        assert!(p.sql.contains("IN"));
        assert_eq!(p.bindings.len(), 2);
    }

    #[test]
    fn in_with_empty_param_array_returns_false() {
        let p = do_plan(r#"*[_id in $ids]"#, json!({"ids": []}));
        assert!(p.sql.contains("FALSE"));
    }

    #[test]
    fn match_glob_compiles_to_ilike() {
        // GROQ `match` is an infix operator: `field match "pattern"`.
        let p = do_plan(r#"*[title match "Hello*"]"#, json!({}));
        assert!(p.sql.contains("ILIKE"));
        assert_eq!(p.bindings.len(), 1);
        // * becomes %
        if let Value::String(pat) = &p.bindings[0] {
            assert!(pat.ends_with('%'));
        } else {
            panic!("expected string binding");
        }
    }

    #[test]
    fn references_function_generates_jsonpath() {
        let p = do_plan(r#"*[references("doc-1")]"#, json!({}));
        assert!(p.sql.contains("jsonb_path_exists"));
        assert!(p.sql.contains("_ref"));
    }

    #[test]
    fn deref_projection_captures_sub_spec() {
        let p = do_plan(r#"*[_type == "page"]{author->name}"#, json!({}));
        let spec = p.projection.expect("projection");
        match &spec.fields[0].1 {
            ProjectionSource::Deref { path, sub_projection } => {
                assert_eq!(path, &vec!["author".to_string()]);
                assert!(sub_projection.is_some());
            }
            other => panic!("expected Deref, got {other:?}"),
        }
    }

    #[test]
    fn glob_translation() {
        assert_eq!(glob_to_ilike("*foo?bar"), "%foo_bar");
        assert_eq!(glob_to_ilike("50%_off"), "50\\%\\_off");
    }
}
