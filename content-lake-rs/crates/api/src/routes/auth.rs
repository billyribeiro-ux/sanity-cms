//! Real JWT-based auth endpoints.
//!
//! - `POST /v1/auth/login/password` — exchange email+password for a JWT.
//! - `GET  /v1/users/me`            — return the user encoded in the bearer token.
//! - `GET  /v1/users/{id}`          — look up any user by id (auth required).
//! - `POST /v1/auth/logout`         — no-op (JWTs are stateless).
//! - `GET  /v1/auth/providers`      — advertises the single "password" provider.

use axum::{
    extract::{Path, State},
    http::HeaderMap,
    routing::{get, post},
    Extension, Json, Router,
};
use content_lake_core::auth as core_auth;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::{
    error::{ApiError, ApiResult},
    middleware::auth::{issue_token, verify_token, CurrentUser},
    state::AppState,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/auth/login/password", post(login_password))
        .route("/v1/auth/logout", post(auth_logout))
        .route("/v1/auth/providers", get(auth_providers))
        .route("/v1/users/me", get(users_me))
        .route("/v1/users/{id}", get(users_by_id))
}

#[derive(Debug, Deserialize)]
struct LoginRequest {
    email: String,
    password: String,
}

#[derive(Debug, Serialize)]
struct UserPayload {
    id: String,
    name: String,
    email: String,
    #[serde(rename = "profileImage")]
    profile_image: Option<String>,
    role: String,
    provider: &'static str,
    #[serde(rename = "providerId")]
    provider_id: String,
}

impl UserPayload {
    fn from_core(u: &core_auth::User) -> Self {
        Self {
            id: u.id.to_string(),
            name: u.name.clone(),
            email: u.email.clone(),
            profile_image: None,
            role: u.role.clone(),
            provider: "password",
            provider_id: u.email.clone(),
        }
    }

    fn synthetic_admin() -> Self {
        Self {
            id: Uuid::nil().to_string(),
            name: "Admin".to_string(),
            email: "admin@localhost".to_string(),
            profile_image: None,
            role: "administrator".to_string(),
            provider: "password",
            provider_id: "admin".to_string(),
        }
    }
}

/// `POST /v1/auth/login/password` — verify credentials and return a JWT.
async fn login_password(
    State(state): State<AppState>,
    Json(body): Json<LoginRequest>,
) -> ApiResult<Json<Value>> {
    let user = core_auth::verify_password(state.pool(), &body.email, &body.password)
        .await
        .map_err(|e| ApiError::Internal(format!("auth backend error: {e}")))?
        .ok_or(ApiError::Unauthorized)?;

    let token = issue_token(&state.config().jwt_secret, user.id, &user.email, &user.role)?;

    Ok(Json(json!({
        "token": token,
        "user": UserPayload::from_core(&user),
    })))
}

/// `POST /v1/auth/logout` — stateless, always succeeds.
async fn auth_logout() -> Json<Value> {
    Json(json!({ "status": "loggedOut" }))
}

/// `GET /v1/auth/providers` — Studio login screen configuration.
async fn auth_providers() -> Json<Value> {
    Json(json!({
        "providers": [
            {
                "name": "password",
                "title": "Email / password",
                "url": "/v1/auth/login/password",
            }
        ],
        "thirdPartyLogin": false,
        "sso": { "saml": false },
        "loginMethod": "dual",
    }))
}

/// `GET /v1/users/me` — return the user identified by the bearer token.
///
/// This route is in the auth-exempt prefix (`/v1/auth/*` is exempt but
/// `/v1/users/me` is not), so requests without a valid JWT are rejected. We
/// still resolve the header ourselves so this works even when the global
/// middleware isn't applied (e.g. unit tests or auth-disabled mode).
async fn users_me(
    State(state): State<AppState>,
    headers: HeaderMap,
    current: Option<Extension<CurrentUser>>,
) -> ApiResult<Json<Value>> {
    // If middleware attached a CurrentUser (either from JWT or AUTH_DISABLED),
    // prefer that. Otherwise, try to resolve the bearer ourselves.
    let (user_id, email_from_token, role_from_token) = if let Some(Extension(cur)) = current {
        (cur.id, cur.email.clone(), cur.role.clone())
    } else {
        // Fallback: parse the header directly. Avoids a 401 when this route is
        // hit without the middleware chain (defensive).
        let token = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| {
                s.strip_prefix("Bearer ")
                    .or_else(|| s.strip_prefix("bearer "))
            })
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or(ApiError::Unauthorized)?;
        let claims = verify_token(&state.config().jwt_secret, token)?;
        let id = Uuid::parse_str(&claims.sub).map_err(|_| ApiError::Unauthorized)?;
        (id, claims.email, claims.role)
    };

    // If AUTH_DISABLED produced the synthetic admin (nil UUID), respond with
    // the synthetic payload — no database lookup needed.
    if user_id.is_nil() {
        return Ok(Json(
            serde_json::to_value(UserPayload::synthetic_admin()).unwrap(),
        ));
    }

    match core_auth::get_user(state.pool(), user_id)
        .await
        .map_err(|e| ApiError::Internal(format!("user lookup failed: {e}")))?
    {
        Some(u) => Ok(Json(
            serde_json::to_value(UserPayload::from_core(&u)).unwrap(),
        )),
        None => {
            // Token is cryptographically valid but the user no longer exists.
            // Fall back to the token's own claims so Studio can still render,
            // but mark the id/email consistently.
            Ok(Json(json!({
                "id": user_id.to_string(),
                "name": email_from_token,
                "email": email_from_token,
                "profileImage": null,
                "role": role_from_token,
                "provider": "password",
                "providerId": email_from_token,
            })))
        }
    }
}

/// `GET /v1/users/{id}` — look up a user by id.
async fn users_by_id(
    State(state): State<AppState>,
    Path(id): Path<String>,
    current: Option<Extension<CurrentUser>>,
) -> ApiResult<Json<Value>> {
    // Require an authenticated caller (either via middleware or not; this
    // route is expected to be protected by the middleware).
    if current.is_none() && !state.config().auth_disabled {
        return Err(ApiError::Unauthorized);
    }

    let uuid = Uuid::parse_str(&id).map_err(|_| ApiError::NotFound(id.clone()))?;

    match core_auth::get_user(state.pool(), uuid)
        .await
        .map_err(|e| ApiError::Internal(format!("user lookup failed: {e}")))?
    {
        Some(u) => Ok(Json(
            serde_json::to_value(UserPayload::from_core(&u)).unwrap(),
        )),
        None => Err(ApiError::NotFound(format!("user {id} not found"))),
    }
}
