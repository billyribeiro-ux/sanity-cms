use serde::{Deserialize, Serialize};

/// GROQ Abstract Syntax Tree types.

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Expr {
    // Literals
    StringLiteral(String),
    IntLiteral(i64),
    FloatLiteral(f64),
    BoolLiteral(bool),
    Null,
    Array(Vec<Expr>),

    // Identifiers & access
    Ident(String),
    DotAccess(Box<Expr>, String),
    Deref(Box<Expr>, String),
    This,
    Parent,

    // Comparison operators
    Eq(Box<Expr>, Box<Expr>),
    Neq(Box<Expr>, Box<Expr>),
    Lt(Box<Expr>, Box<Expr>),
    Gt(Box<Expr>, Box<Expr>),
    Lte(Box<Expr>, Box<Expr>),
    Gte(Box<Expr>, Box<Expr>),
    In(Box<Expr>, Box<Expr>),

    // Logical operators
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Not(Box<Expr>),

    // Query constructs
    Everything,
    Filter(Box<Expr>),
    Projection(Vec<(String, Expr)>),
    Pipeline(Vec<Expr>),
    Order(Box<Expr>, bool),
    Slice(Box<Expr>, i64, i64),

    // Function call
    FuncCall(String, Vec<Expr>),

    // Parameter reference ($param)
    Param(String),
}
