// GROQ in-memory evaluator (for grant filter evaluation).
// Will be fully implemented in Phase 2.

use serde_json::Value;
use crate::ast::Expr;

#[derive(Debug, thiserror::Error)]
pub enum EvalError {
    #[error("type error: {0}")]
    TypeError(String),
    #[error("unsupported expression")]
    Unsupported,
}

pub fn eval_filter(expr: &Expr, doc: &Value, params: &Value) -> Result<bool, EvalError> {
    match eval_expr(expr, doc, params)? {
        Value::Bool(b) => Ok(b),
        _ => Ok(false),
    }
}

pub fn eval_expr(expr: &Expr, doc: &Value, params: &Value) -> Result<Value, EvalError> {
    match expr {
        Expr::Everything => Ok(Value::Bool(true)),
        Expr::BoolLiteral(b) => Ok(Value::Bool(*b)),
        Expr::IntLiteral(n) => Ok(Value::Number((*n).into())),
        Expr::StringLiteral(s) => Ok(Value::String(s.clone())),
        Expr::Null => Ok(Value::Null),
        Expr::Ident(name) => Ok(doc.get(name).cloned().unwrap_or(Value::Null)),
        Expr::DotAccess(base, field) => {
            let v = eval_expr(base, doc, params)?;
            Ok(v.get(field).cloned().unwrap_or(Value::Null))
        }
        Expr::Param(name) => Ok(params.get(name).cloned().unwrap_or(Value::Null)),
        Expr::This => Ok(doc.clone()),
        Expr::Eq(l, r) => {
            let lv = eval_expr(l, doc, params)?;
            let rv = eval_expr(r, doc, params)?;
            Ok(Value::Bool(lv == rv))
        }
        Expr::Neq(l, r) => {
            let lv = eval_expr(l, doc, params)?;
            let rv = eval_expr(r, doc, params)?;
            Ok(Value::Bool(lv != rv))
        }
        Expr::And(l, r) => {
            Ok(Value::Bool(eval_filter(l, doc, params)? && eval_filter(r, doc, params)?))
        }
        Expr::Or(l, r) => {
            Ok(Value::Bool(eval_filter(l, doc, params)? || eval_filter(r, doc, params)?))
        }
        Expr::Not(inner) => {
            Ok(Value::Bool(!eval_filter(inner, doc, params)?))
        }
        _ => Err(EvalError::Unsupported),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn eval_simple_eq() {
        let expr = Expr::Eq(
            Box::new(Expr::Ident("_type".into())),
            Box::new(Expr::StringLiteral("post".into())),
        );
        let doc = json!({"_type": "post"});
        assert!(eval_filter(&expr, &doc, &json!({})).unwrap());
    }

    #[test]
    fn eval_and() {
        let expr = Expr::And(
            Box::new(Expr::Eq(
                Box::new(Expr::Ident("_type".into())),
                Box::new(Expr::StringLiteral("post".into())),
            )),
            Box::new(Expr::Eq(
                Box::new(Expr::Ident("published".into())),
                Box::new(Expr::BoolLiteral(true)),
            )),
        );
        let doc = json!({"_type": "post", "published": true});
        assert!(eval_filter(&expr, &doc, &json!({})).unwrap());
    }

    #[test]
    fn eval_dot_access() {
        let expr = Expr::Eq(
            Box::new(Expr::DotAccess(
                Box::new(Expr::Ident("author".into())),
                "_ref".into(),
            )),
            Box::new(Expr::StringLiteral("user1".into())),
        );
        let doc = json!({"author": {"_ref": "user1"}});
        assert!(eval_filter(&expr, &doc, &json!({})).unwrap());
    }
}
