use axum::{
    extract::Path,
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};

use crate::state::AppState;

/// The stub admin user id returned for all user lookups in the single-user MVP.
const ADMIN_USER_ID: &str = "p-admin";

/// Auth + user stub routes.
///
/// Real auth (sessions, password hashing, JWT issuance) is deferred to Phase 1.5.
/// For now, these endpoints exist so Sanity Studio can boot without a live
/// identity service.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/users/me", get(users_me))
        .route("/v1/users/{id}", get(users_by_id))
        .route("/v1/auth/providers", get(auth_providers))
        .route("/v1/auth/logout", post(auth_logout))
}

/// Build the canonical admin user payload used by `/v1/users/me` and
/// `/v1/users/:id`. We construct at call time (rather than a `static`) so that
/// callers can freely mutate the returned `Value` without any shared state.
fn admin_user() -> Value {
    json!({
        "id": ADMIN_USER_ID,
        "name": "Admin",
        "email": "admin@localhost",
        "profileImage": null,
        "role": "administrator",
        "provider": "password",
        "providerId": "admin",
    })
}

/// `GET /v1/users/me` — Studio polls this on boot to decide if a session is
/// valid. In the single-user MVP we always return the admin user regardless of
/// whether an `Authorization` header was sent.
async fn users_me() -> Json<Value> {
    Json(admin_user())
}

/// `GET /v1/users/:id` — Studio requests user data to display collaborators.
/// For the stub, any id resolves to the same admin user.
async fn users_by_id(Path(_id): Path<String>) -> Json<Value> {
    Json(admin_user())
}

/// `GET /v1/auth/providers` — used by Studio to render the login screen.
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

/// `POST /v1/auth/logout` — no-op in the stub; we simply acknowledge.
async fn auth_logout() -> Json<Value> {
    Json(json!({ "status": "loggedOut" }))
}
