/// Document validation utilities.
/// Will be expanded in Phase 1.
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ValidationError {
    #[error("document _id is required")]
    MissingId,
    #[error("document _type is required")]
    MissingType,
    #[error("document _id cannot be empty")]
    EmptyId,
    #[error("document _type cannot be empty")]
    EmptyType,
}

/// Validate that a document has the minimum required fields.
pub fn validate_document_fields(
    id: Option<&str>,
    doc_type: Option<&str>,
) -> Result<(), ValidationError> {
    match id {
        None => return Err(ValidationError::MissingId),
        Some("") => return Err(ValidationError::EmptyId),
        _ => {}
    }
    match doc_type {
        None => return Err(ValidationError::MissingType),
        Some("") => return Err(ValidationError::EmptyType),
        _ => {}
    }
    Ok(())
}
