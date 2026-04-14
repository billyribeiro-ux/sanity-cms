//! Schema validation error types.
//!
//! Split into two families:
//! - `SchemaError` — problems parsing the schema JSON at startup.
//! - `ValidationError` — problems validating an actual document at mutate time.

use thiserror::Error;

/// Parse-time errors. Returned from `SchemaRegistry::from_json` /
/// `from_file`. These surface as a hard startup failure — the operator
/// should see them immediately in the logs when the server boots.
#[derive(Debug, Error)]
pub enum SchemaError {
    #[error("schema root must be a JSON object")]
    NotAnObject,

    #[error("schema is missing the required `types` object")]
    MissingTypes,

    #[error("type `{type_name}` is not a JSON object")]
    TypeNotObject { type_name: String },

    #[error("type `{type_name}` field at index {index} is not a JSON object")]
    FieldNotObject { type_name: String, index: usize },

    #[error("type `{type_name}` field at index {index} is missing `name`")]
    FieldMissingName { type_name: String, index: usize },

    #[error("field `{path}` has no `type`")]
    FieldMissingType { path: String },

    #[error("field `{path}` has unknown primitive type `{ty}`")]
    UnknownPrimitive { path: String, ty: String },

    #[error("field `{path}`: invalid regex `{pattern}`: {source}")]
    InvalidRegex {
        path: String,
        pattern: String,
        #[source]
        source: regex::Error,
    },

    #[error("field `{path}`: `of` must be an array of strings")]
    InvalidOfList { path: String },

    #[error("failed to read schema file `{path}`: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to parse schema JSON: {0}")]
    Json(#[from] serde_json::Error),

    #[error("field `{path}`: `fields` must be an array")]
    FieldsNotArray { path: String },

    #[error("field `{path}`: object type requires a `fields` array")]
    ObjectMissingFields { path: String },
}

/// Runtime errors produced by `SchemaRegistry::validate_document`.
///
/// The `path` is a JSONPath-like string rooted at `$` pointing at the
/// offending value so operators can quickly see where validation failed.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum ValidationError {
    #[error("at path {path}: missing required field")]
    MissingRequired { path: String },

    #[error("at path {path}: expected {expected}, got {got}")]
    TypeMismatch {
        path: String,
        expected: String,
        got: String,
    },

    #[error("at path {path}: value does not match expected constant")]
    ValueMismatch { path: String },

    #[error("at path {path}: string length {actual} below minimum {min}")]
    StringTooShort {
        path: String,
        actual: usize,
        min: usize,
    },

    #[error("at path {path}: string length {actual} above maximum {max}")]
    StringTooLong {
        path: String,
        actual: usize,
        max: usize,
    },

    #[error("at path {path}: array length {actual} below minimum {min}")]
    ArrayTooShort {
        path: String,
        actual: usize,
        min: usize,
    },

    #[error("at path {path}: array length {actual} above maximum {max}")]
    ArrayTooLong {
        path: String,
        actual: usize,
        max: usize,
    },

    #[error("at path {path}: number {actual} below minimum {min}")]
    NumberTooSmall { path: String, actual: f64, min: f64 },

    #[error("at path {path}: number {actual} above maximum {max}")]
    NumberTooLarge { path: String, actual: f64, max: f64 },

    #[error("at path {path}: string does not match pattern /{pattern}/")]
    PatternMismatch { path: String, pattern: String },

    #[error("at path {path}: value is not a valid ISO 8601 datetime")]
    InvalidDatetime { path: String },

    #[error("at path {path}: reference must be {{_ref, _type}} with string values")]
    InvalidReference { path: String },

    #[error(
        "at path {path}: image must be an object with `_type: \"image\"` and `asset` reference"
    )]
    InvalidImage { path: String },

    #[error(
        "at path {path}: block must be an object with `_type: \"block\"` and a `children` array"
    )]
    InvalidBlock { path: String },

    #[error("at path {path}: array item does not match any of the allowed types {allowed:?}")]
    ArrayItemMismatch { path: String, allowed: Vec<String> },

    #[error("schema error: alias `{0}` refers to an unknown type")]
    UnknownAlias(String),
}
