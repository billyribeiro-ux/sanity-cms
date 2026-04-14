//! Server-side document schema validation.
//!
//! Studios validate documents client-side, but the backend doesn't trust the
//! wire — anyone with a JWT can persist `{"_type": "page", "garbage": "goes"}`.
//! This module loads a JSON schema registry at startup and validates every
//! document produced by the mutation engine before it reaches the repo.
//!
//! ## JSON schema format
//!
//! ```json
//! {
//!   "types": {
//!     "page": {
//!       "fields": [
//!         {"name": "_type", "type": "string", "value": "page"},
//!         {"name": "title", "type": "string", "required": true, "max": 120},
//!         {"name": "slug",  "type": "object", "required": true,
//!          "fields": [{"name": "current", "type": "string", "required": true}]},
//!         {"name": "body",  "type": "array", "of": ["block", "imageWithAlt"]}
//!       ]
//!     }
//!   }
//! }
//! ```
//!
//! Opt-in by setting `SCHEMA_FILE=/path/to/schema.json`. When unset, the
//! registry is empty — validation is a no-op.

pub mod error;

use regex::Regex;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::path::Path;

pub use error::{SchemaError, ValidationError};

/// A read-only registry of known document types. Built once at startup.
#[derive(Debug, Default)]
pub struct SchemaRegistry {
    types: HashMap<String, TypeDef>,
}

#[derive(Debug, Clone)]
pub enum TypeDef {
    Primitive(PrimitiveType),
    Object(ObjectType),
    Array(ArrayType),
    Reference,
    Block,
    Image,
    /// `{type: "someOtherType"}` — look up by name in the registry at validate time.
    Alias(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrimitiveType {
    String,
    Number,
    Boolean,
    Null,
    Datetime,
}

#[derive(Debug, Clone)]
pub struct ObjectType {
    pub fields: Vec<FieldDef>,
}

#[derive(Debug, Clone)]
pub struct ArrayType {
    /// The list of type names accepted as elements. An array item that
    /// matches ANY of these is accepted.
    pub allowed: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct FieldDef {
    pub name: String,
    pub type_def: TypeDef,
    pub required: bool,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub exact_value: Option<Value>,
    pub pattern: Option<Regex>,
}

impl SchemaRegistry {
    pub fn empty() -> Self {
        Self::default()
    }

    /// Number of registered types — log at startup.
    pub fn len(&self) -> usize {
        self.types.len()
    }

    /// Whether any validation will actually happen.
    pub fn is_empty(&self) -> bool {
        self.types.is_empty()
    }

    pub fn from_json(json: &Value) -> Result<Self, SchemaError> {
        let root = json.as_object().ok_or(SchemaError::NotAnObject)?;
        let types_obj = root
            .get("types")
            .and_then(|v| v.as_object())
            .ok_or(SchemaError::MissingTypes)?;

        let mut types: HashMap<String, TypeDef> = HashMap::with_capacity(types_obj.len());
        for (type_name, def) in types_obj {
            let obj = def.as_object().ok_or_else(|| SchemaError::TypeNotObject {
                type_name: type_name.clone(),
            })?;
            let type_def = parse_type_top(type_name, obj)?;
            types.insert(type_name.clone(), type_def);
        }
        Ok(Self { types })
    }

    pub fn from_file(path: &Path) -> Result<Self, SchemaError> {
        let bytes = std::fs::read(path).map_err(|e| SchemaError::Io {
            path: path.display().to_string(),
            source: e,
        })?;
        let json: Value = serde_json::from_slice(&bytes)?;
        Self::from_json(&json)
    }

    /// Build a registry from the `SCHEMA_FILE` env var. Missing env → empty.
    pub fn from_env_or_empty() -> Self {
        match std::env::var("SCHEMA_FILE") {
            Ok(p) if !p.is_empty() => match Self::from_file(Path::new(&p)) {
                Ok(r) => r,
                Err(e) => {
                    tracing::error!(error = %e, path = %p, "failed to load schema file; running with no validation");
                    Self::default()
                }
            },
            _ => Self::default(),
        }
    }

    /// Validate a fully-materialized document (must contain `_id` and `_type`).
    /// Documents whose `_type` isn't in the registry pass through silently —
    /// this keeps validation opt-in on a per-type basis.
    pub fn validate_document(&self, doc: &Value) -> Result<(), ValidationError> {
        if self.types.is_empty() {
            return Ok(());
        }
        let doc_type = doc
            .get("_type")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let Some(def) = self.types.get(doc_type) else {
            return Ok(());
        };
        let TypeDef::Object(ot) = def else {
            return Ok(());
        };
        validate_object(self, ot, doc, "$")
    }
}

// ---- parser ----

fn parse_type_top(type_name: &str, obj: &Map<String, Value>) -> Result<TypeDef, SchemaError> {
    // A top-level type block always has "fields" and represents an object.
    let fields_val = obj
        .get("fields")
        .ok_or_else(|| SchemaError::ObjectMissingFields {
            path: type_name.to_string(),
        })?;
    let fields_arr = fields_val.as_array().ok_or_else(|| SchemaError::FieldsNotArray {
        path: type_name.to_string(),
    })?;
    let fields = parse_fields(type_name, fields_arr)?;
    Ok(TypeDef::Object(ObjectType { fields }))
}

fn parse_fields(path: &str, items: &[Value]) -> Result<Vec<FieldDef>, SchemaError> {
    let mut out = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let m = item.as_object().ok_or_else(|| SchemaError::FieldNotObject {
            type_name: path.to_string(),
            index: i,
        })?;
        let name = m
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| SchemaError::FieldMissingName {
                type_name: path.to_string(),
                index: i,
            })?
            .to_string();
        let field_path = format!("{path}.{name}");

        let type_def = parse_inline_type(&field_path, m)?;

        let required = m.get("required").and_then(|v| v.as_bool()).unwrap_or(false);
        let min = m.get("min").and_then(|v| v.as_f64());
        let max = m.get("max").and_then(|v| v.as_f64());
        let exact_value = m.get("value").cloned();
        let pattern = m
            .get("pattern")
            .and_then(|v| v.as_str())
            .map(|p| Regex::new(p).map_err(|source| SchemaError::InvalidRegex {
                path: field_path.clone(),
                pattern: p.to_string(),
                source,
            }))
            .transpose()?;

        out.push(FieldDef {
            name,
            type_def,
            required,
            min,
            max,
            exact_value,
            pattern,
        });
    }
    Ok(out)
}

fn parse_inline_type(path: &str, m: &Map<String, Value>) -> Result<TypeDef, SchemaError> {
    let ty = m
        .get("type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| SchemaError::FieldMissingType {
            path: path.to_string(),
        })?;

    match ty {
        "string" => {
            // string with format: datetime is equivalent to datetime primitive
            if m.get("format").and_then(|v| v.as_str()) == Some("datetime") {
                Ok(TypeDef::Primitive(PrimitiveType::Datetime))
            } else {
                Ok(TypeDef::Primitive(PrimitiveType::String))
            }
        }
        "number" => Ok(TypeDef::Primitive(PrimitiveType::Number)),
        "boolean" => Ok(TypeDef::Primitive(PrimitiveType::Boolean)),
        "null" => Ok(TypeDef::Primitive(PrimitiveType::Null)),
        "datetime" => Ok(TypeDef::Primitive(PrimitiveType::Datetime)),
        "reference" => Ok(TypeDef::Reference),
        "block" => Ok(TypeDef::Block),
        "image" => Ok(TypeDef::Image),
        "object" => {
            let arr = m
                .get("fields")
                .and_then(|v| v.as_array())
                .ok_or_else(|| SchemaError::ObjectMissingFields {
                    path: path.to_string(),
                })?;
            let fields = parse_fields(path, arr)?;
            Ok(TypeDef::Object(ObjectType { fields }))
        }
        "array" => {
            let arr = m.get("of").and_then(|v| v.as_array());
            let allowed = match arr {
                Some(items) => {
                    let mut a = Vec::with_capacity(items.len());
                    for it in items {
                        let s = it.as_str().ok_or_else(|| SchemaError::InvalidOfList {
                            path: path.to_string(),
                        })?;
                        a.push(s.to_string());
                    }
                    a
                }
                None => Vec::new(),
            };
            Ok(TypeDef::Array(ArrayType { allowed }))
        }
        other => {
            // Unknown name → treat as alias. Resolved at validate time; if the
            // referenced type doesn't exist in the registry we let the value
            // through (unknown alias is a soft failure).
            if other.starts_with(|c: char| c.is_alphabetic()) {
                Ok(TypeDef::Alias(other.to_string()))
            } else {
                Err(SchemaError::UnknownPrimitive {
                    path: path.to_string(),
                    ty: other.to_string(),
                })
            }
        }
    }
}

// ---- validator ----

fn validate_object(
    reg: &SchemaRegistry,
    def: &ObjectType,
    val: &Value,
    path: &str,
) -> Result<(), ValidationError> {
    let obj = match val.as_object() {
        Some(o) => o,
        None => {
            return Err(ValidationError::TypeMismatch {
                path: path.to_string(),
                expected: "object".into(),
                got: json_kind(val).into(),
            });
        }
    };

    for field in &def.fields {
        let fpath = format!("{path}.{}", field.name);
        let child = obj.get(&field.name);

        match child {
            None | Some(Value::Null) => {
                if field.required {
                    return Err(ValidationError::MissingRequired { path: fpath });
                }
            }
            Some(v) => validate_value(reg, field, v, &fpath)?,
        }
    }
    Ok(())
}

fn validate_value(
    reg: &SchemaRegistry,
    field: &FieldDef,
    val: &Value,
    path: &str,
) -> Result<(), ValidationError> {
    // Exact-value constraint first — most constraining.
    if let Some(expected) = &field.exact_value {
        if val != expected {
            return Err(ValidationError::ValueMismatch {
                path: path.to_string(),
            });
        }
    }

    validate_type(reg, &field.type_def, val, path)?;

    // String/number/array constraints.
    match val {
        Value::String(s) => {
            let len = s.chars().count();
            if let Some(min) = field.min {
                if (len as f64) < min {
                    return Err(ValidationError::StringTooShort {
                        path: path.to_string(),
                        actual: len,
                        min: min as usize,
                    });
                }
            }
            if let Some(max) = field.max {
                if (len as f64) > max {
                    return Err(ValidationError::StringTooLong {
                        path: path.to_string(),
                        actual: len,
                        max: max as usize,
                    });
                }
            }
            if let Some(re) = &field.pattern {
                if !re.is_match(s) {
                    return Err(ValidationError::PatternMismatch {
                        path: path.to_string(),
                        pattern: re.as_str().to_string(),
                    });
                }
            }
        }
        Value::Number(n) => {
            let f = n.as_f64().unwrap_or(0.0);
            if let Some(min) = field.min {
                if f < min {
                    return Err(ValidationError::NumberTooSmall {
                        path: path.to_string(),
                        actual: f,
                        min,
                    });
                }
            }
            if let Some(max) = field.max {
                if f > max {
                    return Err(ValidationError::NumberTooLarge {
                        path: path.to_string(),
                        actual: f,
                        max,
                    });
                }
            }
        }
        Value::Array(items) => {
            if let Some(min) = field.min {
                if (items.len() as f64) < min {
                    return Err(ValidationError::ArrayTooShort {
                        path: path.to_string(),
                        actual: items.len(),
                        min: min as usize,
                    });
                }
            }
            if let Some(max) = field.max {
                if (items.len() as f64) > max {
                    return Err(ValidationError::ArrayTooLong {
                        path: path.to_string(),
                        actual: items.len(),
                        max: max as usize,
                    });
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_type(
    reg: &SchemaRegistry,
    def: &TypeDef,
    val: &Value,
    path: &str,
) -> Result<(), ValidationError> {
    match def {
        TypeDef::Primitive(prim) => validate_primitive(*prim, val, path),
        TypeDef::Object(ot) => validate_object(reg, ot, val, path),
        TypeDef::Array(at) => validate_array(reg, at, val, path),
        TypeDef::Reference => validate_reference(val, path),
        TypeDef::Image => validate_image(val, path),
        TypeDef::Block => validate_block(val, path),
        TypeDef::Alias(name) => match reg.types.get(name) {
            Some(t) => validate_type(reg, t, val, path),
            None => Ok(()), // soft failure for unknown aliases
        },
    }
}

fn validate_primitive(prim: PrimitiveType, val: &Value, path: &str) -> Result<(), ValidationError> {
    let ok = match (prim, val) {
        (PrimitiveType::String, Value::String(_)) => true,
        (PrimitiveType::Number, Value::Number(_)) => true,
        (PrimitiveType::Boolean, Value::Bool(_)) => true,
        (PrimitiveType::Null, Value::Null) => true,
        (PrimitiveType::Datetime, Value::String(s)) => is_iso8601(s),
        _ => false,
    };
    if ok {
        Ok(())
    } else if prim == PrimitiveType::Datetime {
        Err(ValidationError::InvalidDatetime {
            path: path.to_string(),
        })
    } else {
        Err(ValidationError::TypeMismatch {
            path: path.to_string(),
            expected: primitive_name(prim).into(),
            got: json_kind(val).into(),
        })
    }
}

fn validate_array(
    reg: &SchemaRegistry,
    def: &ArrayType,
    val: &Value,
    path: &str,
) -> Result<(), ValidationError> {
    let items = match val.as_array() {
        Some(a) => a,
        None => {
            return Err(ValidationError::TypeMismatch {
                path: path.to_string(),
                expected: "array".into(),
                got: json_kind(val).into(),
            });
        }
    };
    if def.allowed.is_empty() {
        return Ok(()); // no element constraint
    }
    for (i, item) in items.iter().enumerate() {
        let ipath = format!("{path}[{i}]");
        let matched = def
            .allowed
            .iter()
            .any(|type_name| array_item_matches(reg, type_name, item));
        if !matched {
            return Err(ValidationError::ArrayItemMismatch {
                path: ipath,
                allowed: def.allowed.clone(),
            });
        }
    }
    Ok(())
}

fn array_item_matches(reg: &SchemaRegistry, type_name: &str, item: &Value) -> bool {
    // Fast paths for built-in names.
    match type_name {
        "string" => return item.is_string(),
        "number" => return item.is_number(),
        "boolean" => return item.is_boolean(),
        "block" => return validate_block(item, "").is_ok(),
        "image" => return validate_image(item, "").is_ok(),
        "reference" => return validate_reference(item, "").is_ok(),
        _ => {}
    }
    // Try registry lookup: if item._type matches a registered name, accept.
    let item_type = item.get("_type").and_then(|v| v.as_str()).unwrap_or("");
    if item_type == type_name {
        // If registered, validate against the def; else accept liberally.
        if let Some(TypeDef::Object(ot)) = reg.types.get(type_name) {
            return validate_object(reg, ot, item, "").is_ok();
        }
        return true;
    }
    false
}

fn validate_reference(val: &Value, path: &str) -> Result<(), ValidationError> {
    let obj = match val.as_object() {
        Some(o) => o,
        None => {
            return Err(ValidationError::InvalidReference {
                path: path.to_string(),
            });
        }
    };
    let _ref_ok = obj.get("_ref").and_then(|v| v.as_str()).is_some();
    // `_type` on a reference is optional in Sanity, so don't require it.
    if _ref_ok {
        Ok(())
    } else {
        Err(ValidationError::InvalidReference {
            path: path.to_string(),
        })
    }
}

fn validate_image(val: &Value, path: &str) -> Result<(), ValidationError> {
    let obj = match val.as_object() {
        Some(o) => o,
        None => {
            return Err(ValidationError::InvalidImage {
                path: path.to_string(),
            });
        }
    };
    if obj.get("_type").and_then(|v| v.as_str()) != Some("image") {
        return Err(ValidationError::InvalidImage {
            path: path.to_string(),
        });
    }
    if let Some(asset) = obj.get("asset") {
        validate_reference(asset, path).map_err(|_| ValidationError::InvalidImage {
            path: path.to_string(),
        })
    } else {
        Err(ValidationError::InvalidImage {
            path: path.to_string(),
        })
    }
}

fn validate_block(val: &Value, path: &str) -> Result<(), ValidationError> {
    let obj = match val.as_object() {
        Some(o) => o,
        None => {
            return Err(ValidationError::InvalidBlock {
                path: path.to_string(),
            });
        }
    };
    if obj.get("_type").and_then(|v| v.as_str()) != Some("block") {
        return Err(ValidationError::InvalidBlock {
            path: path.to_string(),
        });
    }
    if !obj.get("children").map(|v| v.is_array()).unwrap_or(false) {
        return Err(ValidationError::InvalidBlock {
            path: path.to_string(),
        });
    }
    Ok(())
}

// ---- helpers ----

fn primitive_name(p: PrimitiveType) -> &'static str {
    match p {
        PrimitiveType::String => "string",
        PrimitiveType::Number => "number",
        PrimitiveType::Boolean => "boolean",
        PrimitiveType::Null => "null",
        PrimitiveType::Datetime => "datetime",
    }
}

fn json_kind(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Loose ISO 8601 detector — accepts typical `YYYY-MM-DDTHH:MM:SS[.fff]Z` and
/// timezone-offset variants. We delegate to chrono for correctness.
fn is_iso8601(s: &str) -> bool {
    chrono::DateTime::parse_from_rfc3339(s).is_ok()
        || chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S").is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn reg(s: Value) -> SchemaRegistry {
        SchemaRegistry::from_json(&s).unwrap()
    }

    #[test]
    fn empty_registry_accepts_anything() {
        let r = SchemaRegistry::empty();
        assert!(r.validate_document(&json!({"_type":"anything","foo":1})).is_ok());
    }

    #[test]
    fn parse_simple_schema() {
        let r = reg(json!({"types": {"page": {"fields": [
            {"name": "title", "type": "string", "required": true}
        ]}}}));
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn parse_nested_object() {
        let r = reg(json!({"types": {"page": {"fields": [
            {"name": "slug", "type": "object", "fields": [
                {"name": "current", "type": "string"}
            ]}
        ]}}}));
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn parse_array_with_of() {
        let r = reg(json!({"types": {"page": {"fields": [
            {"name": "body", "type": "array", "of": ["block", "image"]}
        ]}}}));
        assert!(r.types.contains_key("page"));
    }

    #[test]
    fn parse_unknown_primitive_is_alias() {
        // Unknown type names are soft aliases; no parse error.
        let r = reg(json!({"types": {"page": {"fields": [
            {"name": "x", "type": "somethingCustom"}
        ]}}}));
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn missing_required_field() {
        let r = reg(json!({"types": {"page": {"fields": [
            {"name": "title", "type": "string", "required": true}
        ]}}}));
        let err = r
            .validate_document(&json!({"_type": "page"}))
            .unwrap_err();
        assert!(matches!(err, ValidationError::MissingRequired { .. }));
    }

    #[test]
    fn wrong_type_rejected() {
        let r = reg(json!({"types": {"page": {"fields": [
            {"name": "title", "type": "string"}
        ]}}}));
        let err = r
            .validate_document(&json!({"_type":"page","title":42}))
            .unwrap_err();
        assert!(matches!(err, ValidationError::TypeMismatch { .. }));
    }

    #[test]
    fn pattern_enforced() {
        let r = reg(json!({"types": {"page": {"fields": [
            {"name": "slug", "type": "string", "pattern": "^[a-z-]+$"}
        ]}}}));
        assert!(r.validate_document(&json!({"_type":"page","slug":"hello"})).is_ok());
        let err = r.validate_document(&json!({"_type":"page","slug":"Hello World"})).unwrap_err();
        assert!(matches!(err, ValidationError::PatternMismatch { .. }));
    }

    #[test]
    fn string_min_max() {
        let r = reg(json!({"types": {"page": {"fields": [
            {"name": "title", "type": "string", "min": 3, "max": 5}
        ]}}}));
        assert!(r.validate_document(&json!({"_type":"page","title":"hey"})).is_ok());
        assert!(matches!(
            r.validate_document(&json!({"_type":"page","title":"hi"})).unwrap_err(),
            ValidationError::StringTooShort { .. }
        ));
        assert!(matches!(
            r.validate_document(&json!({"_type":"page","title":"howdy!"})).unwrap_err(),
            ValidationError::StringTooLong { .. }
        ));
    }

    #[test]
    fn number_min_max() {
        let r = reg(json!({"types": {"page": {"fields": [
            {"name": "n", "type": "number", "min": 0, "max": 10}
        ]}}}));
        assert!(r.validate_document(&json!({"_type":"page","n":5})).is_ok());
        assert!(matches!(
            r.validate_document(&json!({"_type":"page","n":-1})).unwrap_err(),
            ValidationError::NumberTooSmall { .. }
        ));
    }

    #[test]
    fn array_of_multiple_types() {
        let r = reg(json!({"types": {"page": {"fields": [
            {"name": "body", "type": "array", "of": ["string", "number"]}
        ]}}}));
        assert!(r.validate_document(&json!({"_type":"page","body":["a",1]})).is_ok());
        let err = r.validate_document(&json!({"_type":"page","body":[true]})).unwrap_err();
        assert!(matches!(err, ValidationError::ArrayItemMismatch { .. }));
    }

    #[test]
    fn nested_object_validated() {
        let r = reg(json!({"types": {"page": {"fields": [
            {"name": "slug", "type": "object", "required": true, "fields": [
                {"name": "current", "type": "string", "required": true}
            ]}
        ]}}}));
        let err = r.validate_document(&json!({"_type":"page","slug":{}})).unwrap_err();
        assert!(matches!(err, ValidationError::MissingRequired { .. }));
    }

    #[test]
    fn reference_shape() {
        let r = reg(json!({"types": {"page": {"fields": [
            {"name": "author", "type": "reference"}
        ]}}}));
        assert!(r.validate_document(&json!({"_type":"page","author":{"_ref":"a","_type":"user"}})).is_ok());
        assert!(r.validate_document(&json!({"_type":"page","author":{"no":"ref"}})).is_err());
    }

    #[test]
    fn datetime_format() {
        let r = reg(json!({"types": {"page": {"fields": [
            {"name": "at", "type": "datetime"}
        ]}}}));
        assert!(r.validate_document(&json!({"_type":"page","at":"2026-04-14T00:00:00Z"})).is_ok());
        assert!(matches!(
            r.validate_document(&json!({"_type":"page","at":"not a date"})).unwrap_err(),
            ValidationError::InvalidDatetime { .. }
        ));
    }

    #[test]
    fn exact_value_for_type() {
        // A doc typed "page" with its own _type = "page" validates cleanly.
        // Note: docs with _type != any registered type pass through entirely
        // (validation is opt-in per _type), so we can only verify the happy
        // path here.
        let r = reg(json!({"types": {"page": {"fields": [
            {"name": "_type", "type": "string", "value": "page"},
            {"name": "status", "type": "string", "value": "published"}
        ]}}}));
        assert!(r.validate_document(&json!({"_type":"page","status":"published"})).is_ok());
        assert!(matches!(
            r.validate_document(&json!({"_type":"page","status":"draft"})).unwrap_err(),
            ValidationError::ValueMismatch { .. }
        ));
    }

    #[test]
    fn unknown_type_passes_through() {
        let r = reg(json!({"types": {"page": {"fields": [
            {"name": "title", "type": "string", "required": true}
        ]}}}));
        // "author" isn't registered so we don't validate it.
        assert!(r.validate_document(&json!({"_type":"author","name":"Jane"})).is_ok());
    }

    #[test]
    fn image_and_block_shapes() {
        let r = reg(json!({"types": {"page": {"fields": [
            {"name": "hero", "type": "image"}
        ]}}}));
        assert!(r
            .validate_document(&json!({"_type":"page","hero":{"_type":"image","asset":{"_ref":"x"}}}))
            .is_ok());
        assert!(r
            .validate_document(&json!({"_type":"page","hero":{"_type":"image"}}))
            .is_err());
    }
}
