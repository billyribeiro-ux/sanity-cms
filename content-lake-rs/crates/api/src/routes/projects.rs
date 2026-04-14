use axum::{
    extract::{Path, State},
    routing::get,
    Json, Router,
};
use serde_json::{json, Value};
use sqlx::Row;

use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

/// Project / dataset listing routes.
///
/// These power Studio's project selector and dataset picker. Fields that don't
/// apply to a self-hosted single-tenant deployment (organizations, billing,
/// retention limits) are returned as static placeholders.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/projects/{project_id}", get(get_project))
        .route("/v1/projects/{project_id}/datasets", get(list_datasets))
}

/// `GET /v1/projects/:project_id`
///
/// We treat the public URL segment as the project `name` (unique column),
/// while the internal UUID `id` is also returned for Studio to key on.
///
/// We format `created_at` to RFC 3339 text in SQL so the handler doesn't need
/// to depend on a specific `chrono`/`time` formatting configuration from sqlx.
#[tracing::instrument(skip(state))]
async fn get_project(
    State(state): State<AppState>,
    Path(project_id): Path<String>,
) -> ApiResult<Json<Value>> {
    let row = sqlx::query(
        "SELECT id, name, to_char(created_at AT TIME ZONE 'UTC', \
         'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS created_at_rfc3339 \
         FROM projects WHERE name = $1",
    )
    .bind(&project_id)
    .fetch_optional(state.pool())
    .await?
    .ok_or_else(|| ApiError::NotFound(format!("project '{project_id}' not found")))?;

    let id: uuid::Uuid = row.try_get("id").map_err(ApiError::from)?;
    let name: String = row.try_get("name").map_err(ApiError::from)?;
    let created_at: String = row
        .try_get("created_at_rfc3339")
        .map_err(ApiError::from)?;

    Ok(Json(json!({
        "id": id.to_string(),
        "displayName": name,
        "studioHost": null,
        "isBlocked": false,
        "isDisabled": false,
        "isDisabledByUser": false,
        "metadata": {
            "color": "#000000",
            "externalStudioHost": null,
        },
        "members": [
            {
                "id": "p-admin",
                "role": "administrator",
                "isRobot": false,
                "isCurrentUser": true,
            }
        ],
        "pendingInvites": 0,
        "maxRetentionDays": 30,
        "createdAt": created_at,
        "organizationId": null,
    })))
}

/// `GET /v1/projects/:project_id/datasets`
///
/// Returns all datasets belonging to the project, ordered by creation time for
/// a stable Studio UX. 404s if the project doesn't exist so Studio can
/// distinguish "no datasets" from "no project".
#[tracing::instrument(skip(state))]
async fn list_datasets(
    State(state): State<AppState>,
    Path(project_id): Path<String>,
) -> ApiResult<Json<Value>> {
    // Confirm project exists before listing; otherwise a typo'd project id
    // would silently return [] which is harder to debug.
    let project_row = sqlx::query("SELECT id FROM projects WHERE name = $1")
        .bind(&project_id)
        .fetch_optional(state.pool())
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("project '{project_id}' not found")))?;

    let project_uuid: uuid::Uuid = project_row.try_get("id").map_err(ApiError::from)?;

    let rows = sqlx::query(
        "SELECT name, to_char(created_at AT TIME ZONE 'UTC', \
         'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS created_at_rfc3339 \
         FROM datasets WHERE project_id = $1 ORDER BY created_at ASC",
    )
    .bind(project_uuid)
    .fetch_all(state.pool())
    .await?;

    let datasets: Vec<Value> = rows
        .into_iter()
        .map(|row| {
            let name: String = row.try_get("name").unwrap_or_default();
            let created_at: String = row.try_get("created_at_rfc3339").unwrap_or_default();
            json!({
                "name": name,
                "aclMode": "public",
                "createdAt": created_at,
                "createdByUserId": "p-admin",
                "addonFor": null,
                "datasetProfile": "content",
                "features": [],
                "tags": [],
            })
        })
        .collect();

    Ok(Json(Value::Array(datasets)))
}
