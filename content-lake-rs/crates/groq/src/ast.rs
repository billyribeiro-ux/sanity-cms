/// GROQ Abstract Syntax Tree types.
/// Will be fully implemented in Phase 2.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Expr {
    /// Placeholder — full AST coming in Phase 2
    Everything,
}
