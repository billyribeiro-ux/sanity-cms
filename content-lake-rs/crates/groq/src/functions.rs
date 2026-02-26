// GROQ built-in functions (count, defined, references, etc.).
// Will be fully implemented in Phase 2.

use serde_json::Value;

use crate::eval::EvalError;

/// Evaluate a built-in GROQ function by name.
pub fn call_builtin(name: &str, args: &[Value]) -> Result<Value, EvalError> {
    match name {
        "count" => builtin_count(args),
        "defined" => builtin_defined(args),
        "length" => builtin_length(args),
        "references" => builtin_references(args),
        _ => Err(EvalError::TypeError(format!("unknown function: {name}"))),
    }
}

fn builtin_count(args: &[Value]) -> Result<Value, EvalError> {
    match args.first() {
        Some(Value::Array(arr)) => Ok(Value::Number(arr.len().into())),
        Some(Value::Null) => Ok(Value::Number(0.into())),
        _ => Err(EvalError::TypeError("count() expects an array".into())),
    }
}

fn builtin_defined(args: &[Value]) -> Result<Value, EvalError> {
    match args.first() {
        Some(Value::Null) | None => Ok(Value::Bool(false)),
        _ => Ok(Value::Bool(true)),
    }
}

fn builtin_length(args: &[Value]) -> Result<Value, EvalError> {
    match args.first() {
        Some(Value::String(s)) => Ok(Value::Number(s.len().into())),
        Some(Value::Array(a)) => Ok(Value::Number(a.len().into())),
        _ => Ok(Value::Null),
    }
}

fn builtin_references(args: &[Value]) -> Result<Value, EvalError> {
    if args.len() < 2 {
        return Err(EvalError::TypeError("references() needs 2 args".into()));
    }
    let doc = &args[0];
    let ref_id = match &args[1] {
        Value::String(s) => s,
        _ => return Ok(Value::Bool(false)),
    };
    Ok(Value::Bool(value_references(doc, ref_id)))
}

fn value_references(val: &Value, ref_id: &str) -> bool {
    match val {
        Value::Object(map) => {
            if let Some(Value::String(r)) = map.get("_ref") {
                if r == ref_id {
                    return true;
                }
            }
            map.values().any(|v| value_references(v, ref_id))
        }
        Value::Array(arr) => arr.iter().any(|v| value_references(v, ref_id)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_count() {
        let r = call_builtin("count", &[json!([1, 2, 3])]).unwrap();
        assert_eq!(r, json!(3));
    }

    #[test]
    fn test_defined() {
        assert_eq!(
            call_builtin("defined", &[json!(null)]).unwrap(),
            json!(false)
        );
        assert_eq!(call_builtin("defined", &[json!("x")]).unwrap(), json!(true));
    }

    #[test]
    fn test_length() {
        assert_eq!(call_builtin("length", &[json!("hello")]).unwrap(), json!(5));
        assert_eq!(call_builtin("length", &[json!([1, 2])]).unwrap(), json!(2));
    }

    #[test]
    fn test_references() {
        let doc = json!({"author": {"_ref": "user-1"}, "tags": [{"_ref": "tag-2"}]});
        assert_eq!(
            call_builtin("references", &[doc.clone(), json!("user-1")]).unwrap(),
            json!(true)
        );
        assert_eq!(
            call_builtin("references", &[doc.clone(), json!("nope")]).unwrap(),
            json!(false)
        );
        assert_eq!(
            call_builtin("references", &[doc, json!("tag-2")]).unwrap(),
            json!(true)
        );
    }
}
