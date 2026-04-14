//! JWT-based auth middleware.
//!
//! Verifies `Authorization: Bearer <jwt>` on protected routes and attaches a
//! [`CurrentUser`] extension to the request. When `config.auth_disabled` is
//! true (env `AUTH_DISABLED=1`), a synthetic admin user is attached
//! unconditionally — useful for local development.

use axum::{
    extract::State,
    http::{header, Method, Request},
    middleware::Next,
    response::Response,
};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{error::ApiError, state::AppState};

/// Claims embedded in issued JWTs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    /// User id (UUID as string).
    pub sub: String,
    pub email: String,
    pub role: String,
    /// Expiration, seconds since unix epoch.
    pub exp: i64,
    /// Issued-at, seconds since unix epoch.
    pub iat: i64,
}

/// JWT validity window — 7 days.
pub const JWT_TTL_SECONDS: i64 = 7 * 24 * 60 * 60;

/// The user attached to an authenticated request as an Axum extension.
#[derive(Debug, Clone)]
pub struct CurrentUser {
    pub id: Uuid,
    pub email: String,
    pub role: String,
}

impl CurrentUser {
    /// The synthetic admin user used when auth is disabled.
    pub fn synthetic_admin() -> Self {
        Self {
            id: Uuid::nil(),
            email: "admin@localhost".to_string(),
            role: "administrator".to_string(),
        }
    }
}

/// Sign a JWT for the given user id, email, and role.
pub fn issue_token(
    secret: &str,
    user_id: Uuid,
    email: &str,
    role: &str,
) -> Result<String, ApiError> {
    let now = chrono::Utc::now().timestamp();
    let claims = Claims {
        sub: user_id.to_string(),
        email: email.to_string(),
        role: role.to_string(),
        iat: now,
        exp: now + JWT_TTL_SECONDS,
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| ApiError::Internal(format!("failed to sign JWT: {e}")))
}

/// Verify a JWT and extract its claims. Returns [`ApiError::Unauthorized`] on
/// any failure (bad signature, expired, malformed).
pub fn verify_token(secret: &str, token: &str) -> Result<Claims, ApiError> {
    let validation = Validation::default();
    let data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .map_err(|_| ApiError::Unauthorized)?;
    Ok(data.claims)
}

/// Axum middleware that gates all non-exempt routes on a valid bearer JWT.
///
/// Exempt paths (always pass through without auth):
/// - `/health`, `/v1/ping`
/// - any `/v1/auth/*`
/// - `GET /assets/*`
pub async fn auth_middleware(
    State(state): State<AppState>,
    mut request: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, ApiError> {
    let path = request.uri().path();
    let method = request.method().clone();

    if is_exempt(&method, path) {
        return Ok(next.run(request).await);
    }

    let config = state.config();

    if config.auth_disabled {
        request.extensions_mut().insert(CurrentUser::synthetic_admin());
        return Ok(next.run(request).await);
    }

    let token = bearer_token(request.headers()).ok_or(ApiError::Unauthorized)?;
    let claims = verify_token(&config.jwt_secret, token)?;
    let id = Uuid::parse_str(&claims.sub).map_err(|_| ApiError::Unauthorized)?;
    request.extensions_mut().insert(CurrentUser {
        id,
        email: claims.email,
        role: claims.role,
    });
    Ok(next.run(request).await)
}

fn is_exempt(method: &Method, path: &str) -> bool {
    if path == "/health" || path == "/v1/ping" {
        return true;
    }
    if path.starts_with("/v1/auth/") {
        return true;
    }
    // Static asset serving is public; uploads (POST /v1/assets/...) are protected.
    if method == Method::GET && path.starts_with("/assets/") {
        return true;
    }
    // Read-only document fetch is left open for Studio to boot before login.
    // Mutations go through /v1/data/mutate/... and remain protected.
    if method == Method::GET && path.starts_with("/v1/data/doc/") {
        return true;
    }
    // Project/dataset metadata is read during Studio boot before login lands.
    if method == Method::GET && path.starts_with("/v1/projects") {
        return true;
    }
    // Presence WebSockets use the browser's native WebSocket API which can't
    // send custom Authorization headers. Exempt for MVP; gate via a query-
    // string token in a future phase.
    if path.starts_with("/v1/presence/") {
        return true;
    }
    false
}

fn bearer_token(headers: &axum::http::HeaderMap) -> Option<&str> {
    let raw = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let rest = raw.strip_prefix("Bearer ").or_else(|| raw.strip_prefix("bearer "))?;
    let trimmed = rest.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_token_verifies() {
        let id = Uuid::new_v4();
        let token = issue_token("test-secret", id, "a@b.c", "administrator").unwrap();
        let claims = verify_token("test-secret", &token).unwrap();
        assert_eq!(claims.sub, id.to_string());
        assert_eq!(claims.email, "a@b.c");
        assert_eq!(claims.role, "administrator");
    }

    #[test]
    fn token_with_wrong_secret_is_rejected() {
        let id = Uuid::new_v4();
        let token = issue_token("secret-a", id, "a@b.c", "administrator").unwrap();
        let err = verify_token("secret-b", &token).unwrap_err();
        assert!(matches!(err, ApiError::Unauthorized));
    }

    #[test]
    fn expired_token_is_rejected() {
        // Manually craft a claims set with exp in the past.
        let expired = Claims {
            sub: Uuid::new_v4().to_string(),
            email: "a@b.c".into(),
            role: "administrator".into(),
            iat: 1,
            exp: 100, // Jan 1 1970 + 100s
        };
        let token = jsonwebtoken::encode(
            &Header::default(),
            &expired,
            &EncodingKey::from_secret(b"s"),
        )
        .unwrap();
        let err = verify_token("s", &token).unwrap_err();
        assert!(matches!(err, ApiError::Unauthorized));
    }

    #[test]
    fn exempt_paths_pass_through() {
        assert!(is_exempt(&Method::GET, "/health"));
        assert!(is_exempt(&Method::GET, "/v1/ping"));
        assert!(is_exempt(&Method::POST, "/v1/auth/login/password"));
        assert!(is_exempt(&Method::GET, "/v1/auth/providers"));
        assert!(is_exempt(&Method::GET, "/assets/images/foo.jpg"));
        assert!(is_exempt(&Method::GET, "/v1/data/doc/production/a,b"));
        assert!(is_exempt(&Method::GET, "/v1/projects/default"));
        assert!(!is_exempt(&Method::POST, "/v1/data/mutate/production"));
        assert!(!is_exempt(&Method::GET, "/v1/users/me"));
        assert!(!is_exempt(&Method::POST, "/assets/images/foo.jpg"));
        assert!(!is_exempt(&Method::POST, "/v1/assets/images/production"));
    }

    #[test]
    fn bearer_token_parsing() {
        let mut h = axum::http::HeaderMap::new();
        h.insert(header::AUTHORIZATION, "Bearer abc".parse().unwrap());
        assert_eq!(bearer_token(&h), Some("abc"));
        h.insert(header::AUTHORIZATION, "bearer xyz".parse().unwrap());
        assert_eq!(bearer_token(&h), Some("xyz"));
        h.insert(header::AUTHORIZATION, "Basic foo".parse().unwrap());
        assert_eq!(bearer_token(&h), None);
        h.insert(header::AUTHORIZATION, "Bearer ".parse().unwrap());
        assert_eq!(bearer_token(&h), None);
    }
}
