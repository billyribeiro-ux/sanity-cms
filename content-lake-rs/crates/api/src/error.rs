use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

/// API error type that maps to Sanity-compatible JSON error responses.
#[derive(Debug, thiserror::Error)]
#[allow(dead_code)]
pub enum ApiError {
    #[error("not found: {0}")]
    NotFound(String),

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("unauthorized")]
    Unauthorized,

    #[error("forbidden: {0}")]
    Forbidden(String),

    #[error("conflict: {0}")]
    Conflict(String),

    #[error("internal error: {0}")]
    Internal(String),

    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, error_type, message) = match &self {
            ApiError::NotFound(msg) => (StatusCode::NOT_FOUND, "notFound", msg.clone()),
            ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, "badRequest", msg.clone()),
            ApiError::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "Authentication required".to_string(),
            ),
            ApiError::Forbidden(msg) => (StatusCode::FORBIDDEN, "forbidden", msg.clone()),
            ApiError::Conflict(msg) => (StatusCode::CONFLICT, "conflict", msg.clone()),
            ApiError::Internal(msg) => {
                tracing::error!("Internal error: {msg}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internalError",
                    "An internal error occurred".to_string(),
                )
            }
            ApiError::Database(err) => {
                tracing::error!("Database error: {err}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internalError",
                    "An internal error occurred".to_string(),
                )
            }
        };

        let body = json!({
            "error": {
                "type": error_type,
                "message": message,
                "statusCode": status.as_u16(),
            }
        });

        (status, Json(body)).into_response()
    }
}

/// Convenience type alias for route handlers.
pub type ApiResult<T> = Result<T, ApiError>;
