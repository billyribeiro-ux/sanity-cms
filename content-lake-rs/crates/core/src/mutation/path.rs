//! JSON path utilities for Sanity patch operations.
//!
//! Supports dot-notation with array indexing, e.g. `"a.b[0].c.d[-1]"`.
//!
//! This is intentionally a small, focused implementation of the subset of
//! Sanity's JSONMatch / patch-path grammar that is needed for the mutation
//! engine. Predicate expressions (e.g. `items[_key=="x"]`) are **not**
//! supported here; they will be added in a later phase.

use serde_json::{Map, Value};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PathError {
    #[error("invalid path: {0}")]
    InvalidPath(String),
    #[error("path targets array index but value is not an array: {0}")]
    NotAnArray(String),
    #[error("path targets object field but value is not an object: {0}")]
    NotAnObject(String),
    #[error("index {index} out of bounds for array of length {len}")]
    IndexOutOfBounds { index: i64, len: usize },
    #[error("empty path")]
    Empty,
}

/// A single segment of a parsed path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    /// Object field, e.g. `title`.
    Field(String),
    /// Array index. Negative values count from the end (`-1` = last).
    Index(i64),
}

/// Parse a path string into segments.
///
/// Grammar (informal):
///
/// ```text
/// path      := segment ( "." segment | "[" index "]" )*
/// segment   := field ( "[" index "]" )*
/// field     := [a-zA-Z_][a-zA-Z0-9_]*
/// index     := "-"? [0-9]+
/// ```
pub fn parse_path(path: &str) -> Result<Vec<Segment>, PathError> {
    if path.is_empty() {
        return Err(PathError::Empty);
    }

    let mut segments = Vec::new();
    let bytes = path.as_bytes();
    let mut i = 0;
    let mut expect_separator = false;

    while i < bytes.len() {
        match bytes[i] {
            b'.' => {
                if !expect_separator {
                    return Err(PathError::InvalidPath(path.to_string()));
                }
                expect_separator = false;
                i += 1;
            }
            b'[' => {
                // Find matching ']'
                let end = path[i + 1..]
                    .find(']')
                    .ok_or_else(|| PathError::InvalidPath(path.to_string()))?;
                let inner = &path[i + 1..i + 1 + end];
                let idx: i64 = inner
                    .parse()
                    .map_err(|_| PathError::InvalidPath(path.to_string()))?;
                segments.push(Segment::Index(idx));
                i += end + 2;
                expect_separator = true;
            }
            c if is_field_start(c) => {
                let start = i;
                while i < bytes.len() && is_field_continue(bytes[i]) {
                    i += 1;
                }
                segments.push(Segment::Field(path[start..i].to_string()));
                expect_separator = true;
            }
            _ => return Err(PathError::InvalidPath(path.to_string())),
        }
    }

    if segments.is_empty() {
        return Err(PathError::Empty);
    }

    Ok(segments)
}

fn is_field_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_'
}

fn is_field_continue(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// Resolve a possibly negative array index against a length.
fn resolve_index(idx: i64, len: usize) -> Option<usize> {
    if idx >= 0 {
        let u = idx as usize;
        if u < len {
            Some(u)
        } else {
            None
        }
    } else {
        let abs = (-idx) as usize;
        if abs == 0 || abs > len {
            None
        } else {
            Some(len - abs)
        }
    }
}

/// Look up a value at the given path, returning a reference.
/// Returns `None` if any segment misses.
pub fn get_value<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    let segments = parse_path(path).ok()?;
    get_by_segments(root, &segments)
}

pub fn get_by_segments<'a>(root: &'a Value, segments: &[Segment]) -> Option<&'a Value> {
    let mut cur = root;
    for seg in segments {
        cur = match (seg, cur) {
            (Segment::Field(name), Value::Object(map)) => map.get(name)?,
            (Segment::Index(i), Value::Array(arr)) => {
                let idx = resolve_index(*i, arr.len())?;
                arr.get(idx)?
            }
            _ => return None,
        };
    }
    Some(cur)
}

pub fn get_by_segments_mut<'a>(root: &'a mut Value, segments: &[Segment]) -> Option<&'a mut Value> {
    let mut cur = root;
    for seg in segments {
        cur = match (seg, cur) {
            (Segment::Field(name), Value::Object(map)) => map.get_mut(name)?,
            (Segment::Index(i), Value::Array(arr)) => {
                let idx = resolve_index(*i, arr.len())?;
                arr.get_mut(idx)?
            }
            _ => return None,
        };
    }
    Some(cur)
}

/// Set a value at the given path, creating intermediate objects as needed.
///
/// Intermediate arrays are **not** created; if a segment requires an array
/// index but the current value is missing, an error is returned.
pub fn set_value(root: &mut Value, path: &str, value: Value) -> Result<(), PathError> {
    let segments = parse_path(path)?;
    set_by_segments(root, &segments, value)
}

pub fn set_by_segments(
    root: &mut Value,
    segments: &[Segment],
    value: Value,
) -> Result<(), PathError> {
    if segments.is_empty() {
        return Err(PathError::Empty);
    }

    let (last, prefix) = segments.split_last().unwrap();

    let parent = traverse_mut_create(root, prefix)?;
    match (last, parent) {
        (Segment::Field(name), Value::Object(map)) => {
            map.insert(name.clone(), value);
            Ok(())
        }
        (Segment::Field(_), _) => Err(PathError::NotAnObject(describe_segments(prefix))),
        (Segment::Index(i), Value::Array(arr)) => {
            let idx = resolve_index(*i, arr.len()).ok_or(PathError::IndexOutOfBounds {
                index: *i,
                len: arr.len(),
            })?;
            arr[idx] = value;
            Ok(())
        }
        (Segment::Index(_), _) => Err(PathError::NotAnArray(describe_segments(prefix))),
    }
}

/// Walk the prefix, creating object nodes along the way. Arrays are not
/// auto-created; a missing array index segment is an error.
fn traverse_mut_create<'a>(
    root: &'a mut Value,
    segments: &[Segment],
) -> Result<&'a mut Value, PathError> {
    let mut cur = root;
    for (i, seg) in segments.iter().enumerate() {
        match seg {
            Segment::Field(name) => {
                if !cur.is_object() {
                    return Err(PathError::NotAnObject(describe_segments(&segments[..i])));
                }
                let map = cur.as_object_mut().unwrap();
                if !map.contains_key(name) {
                    map.insert(name.clone(), Value::Object(Map::new()));
                }
                cur = map.get_mut(name).unwrap();
            }
            Segment::Index(n) => {
                let arr = cur
                    .as_array_mut()
                    .ok_or_else(|| PathError::NotAnArray(describe_segments(&segments[..i])))?;
                let idx = resolve_index(*n, arr.len()).ok_or(PathError::IndexOutOfBounds {
                    index: *n,
                    len: arr.len(),
                })?;
                cur = &mut arr[idx];
            }
        }
    }
    Ok(cur)
}

/// Remove the value at the given path. Returns `true` if something was
/// actually removed.
pub fn unset_value(root: &mut Value, path: &str) -> Result<bool, PathError> {
    let segments = parse_path(path)?;
    if segments.is_empty() {
        return Err(PathError::Empty);
    }
    let (last, prefix) = segments.split_last().unwrap();

    // If the parent does not exist, treat as no-op success.
    let parent_opt = get_by_segments_mut(root, prefix);
    let Some(parent) = parent_opt else {
        return Ok(false);
    };

    match (last, parent) {
        (Segment::Field(name), Value::Object(map)) => Ok(map.remove(name).is_some()),
        (Segment::Field(_), _) => Err(PathError::NotAnObject(describe_segments(prefix))),
        (Segment::Index(i), Value::Array(arr)) => {
            if let Some(idx) = resolve_index(*i, arr.len()) {
                arr.remove(idx);
                Ok(true)
            } else {
                Ok(false)
            }
        }
        (Segment::Index(_), _) => Err(PathError::NotAnArray(describe_segments(prefix))),
    }
}

fn describe_segments(segments: &[Segment]) -> String {
    let mut s = String::new();
    for (i, seg) in segments.iter().enumerate() {
        match seg {
            Segment::Field(name) => {
                if i > 0 {
                    s.push('.');
                }
                s.push_str(name);
            }
            Segment::Index(n) => {
                s.push('[');
                s.push_str(&n.to_string());
                s.push(']');
            }
        }
    }
    s
}

/// Split a path into `(parent_segments, last_index)` for insert operations.
/// Returns an error if the last segment is not an array index.
pub fn split_array_path(path: &str) -> Result<(Vec<Segment>, i64), PathError> {
    let segments = parse_path(path)?;
    if segments.is_empty() {
        return Err(PathError::Empty);
    }
    let (last, rest) = segments.split_last().unwrap();
    match last {
        Segment::Index(i) => Ok((rest.to_vec(), *i)),
        Segment::Field(_) => Err(PathError::InvalidPath(format!(
            "expected array index at end of {path}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_simple_field() {
        assert_eq!(
            parse_path("foo").unwrap(),
            vec![Segment::Field("foo".into())]
        );
    }

    #[test]
    fn parse_nested() {
        assert_eq!(
            parse_path("a.b.c").unwrap(),
            vec![
                Segment::Field("a".into()),
                Segment::Field("b".into()),
                Segment::Field("c".into()),
            ]
        );
    }

    #[test]
    fn parse_with_index() {
        assert_eq!(
            parse_path("items[0].name").unwrap(),
            vec![
                Segment::Field("items".into()),
                Segment::Index(0),
                Segment::Field("name".into()),
            ]
        );
    }

    #[test]
    fn parse_negative_index() {
        assert_eq!(
            parse_path("items[-1]").unwrap(),
            vec![Segment::Field("items".into()), Segment::Index(-1)]
        );
    }

    #[test]
    fn parse_empty_rejected() {
        assert!(matches!(parse_path(""), Err(PathError::Empty)));
    }

    #[test]
    fn parse_invalid_rejected() {
        assert!(parse_path("..a").is_err());
        assert!(parse_path("[a]").is_err());
        assert!(parse_path("a[").is_err());
    }

    #[test]
    fn get_nested() {
        let v = json!({"a": {"b": [{"c": 42}]}});
        assert_eq!(get_value(&v, "a.b[0].c"), Some(&json!(42)));
    }

    #[test]
    fn get_negative_index() {
        let v = json!({"a": [1, 2, 3]});
        assert_eq!(get_value(&v, "a[-1]"), Some(&json!(3)));
    }

    #[test]
    fn get_missing() {
        let v = json!({"a": 1});
        assert_eq!(get_value(&v, "b"), None);
    }

    #[test]
    fn set_creates_intermediate() {
        let mut v = json!({});
        set_value(&mut v, "a.b.c", json!(42)).unwrap();
        assert_eq!(v, json!({"a": {"b": {"c": 42}}}));
    }

    #[test]
    fn set_overwrites_existing() {
        let mut v = json!({"a": 1});
        set_value(&mut v, "a", json!(2)).unwrap();
        assert_eq!(v, json!({"a": 2}));
    }

    #[test]
    fn set_array_index() {
        let mut v = json!({"a": [1, 2, 3]});
        set_value(&mut v, "a[1]", json!(99)).unwrap();
        assert_eq!(v, json!({"a": [1, 99, 3]}));
    }

    #[test]
    fn unset_removes_field() {
        let mut v = json!({"a": {"b": 1, "c": 2}});
        assert!(unset_value(&mut v, "a.b").unwrap());
        assert_eq!(v, json!({"a": {"c": 2}}));
    }

    #[test]
    fn unset_missing_returns_false() {
        let mut v = json!({"a": 1});
        assert!(!unset_value(&mut v, "b").unwrap());
    }

    #[test]
    fn unset_array_element() {
        let mut v = json!({"a": [1, 2, 3]});
        assert!(unset_value(&mut v, "a[1]").unwrap());
        assert_eq!(v, json!({"a": [1, 3]}));
    }
}
