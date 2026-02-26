use axum::{extract::State, routing::get, Json, Router};
use serde_json::{json, Value};

use crate::error::ApiResult;
use crate::state::AppState;

/// Health check routes.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/health", get(health_check))
        .route("/v1/ping", get(ping))
}

/// Full health check — verifies database connectivity.
async fn health_check(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    // Verify database connection
    sqlx::query("SELECT 1")
        .execute(state.pool())
        .await
        .map_err(|e| {
            crate::error::ApiError::Internal(format!("database health check failed: {e}"))
        })?;

    Ok(Json(json!({
        "status": "ok",
        "database": "connected",
        "subscribers": state.event_bus().subscriber_count(),
    })))
}

/// Lightweight ping — no database check.
async fn ping() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}
