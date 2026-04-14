//! User accounts and password-based authentication primitives.
//!
//! This module is deliberately IO-light: it exposes a small set of async
//! functions operating on a `PgPool`. The HTTP layer is responsible for
//! translating these into requests/responses and for issuing JWTs.
//!
//! Password hashing uses argon2id with a per-user random salt. Comparison is
//! always constant-time via `PasswordHash::verify_password`.

use anyhow::{anyhow, Context, Result};
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use sqlx::types::time::OffsetDateTime;
use sqlx::PgPool;
use uuid::Uuid;

fn offset_to_chrono(odt: OffsetDateTime) -> DateTime<Utc> {
    let unix_ns = odt.unix_timestamp_nanos();
    let mut secs = (unix_ns.div_euclid(1_000_000_000)) as i64;
    let mut nanos = (unix_ns.rem_euclid(1_000_000_000)) as u32;
    if nanos >= 1_000_000_000 {
        secs += 1;
        nanos -= 1_000_000_000;
    }
    Utc.timestamp_opt(secs, nanos)
        .single()
        .unwrap_or_else(Utc::now)
}

/// A row in the `users` table, minus the password hash.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub name: String,
    pub role: String,
    pub created_at: DateTime<Utc>,
}

/// Compute an argon2id hash suitable for storage in `users.password_hash`.
pub fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow!("argon2 hash failed: {e}"))?
        .to_string();
    Ok(hash)
}

/// Constant-time verification of `password` against a stored argon2 hash.
pub fn verify_password_hash(password: &str, stored_hash: &str) -> Result<bool> {
    let parsed = PasswordHash::new(stored_hash)
        .map_err(|e| anyhow!("stored password hash is malformed: {e}"))?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}

/// Ensure at least one admin user exists. If the `users` table is empty, create
/// a user with the given email + password. Returns the id of the existing or
/// newly-created admin.
pub async fn ensure_admin(pool: &PgPool, email: &str, password: &str) -> Result<Uuid> {
    // If any user exists, do nothing: we don't want to overwrite a real admin.
    let existing: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM users LIMIT 1")
        .fetch_optional(pool)
        .await
        .context("failed to query users table")?;

    if let Some((id,)) = existing {
        return Ok(id);
    }

    let hash = hash_password(password)?;
    let row: (Uuid,) = sqlx::query_as(
        r#"
        INSERT INTO users (email, name, password_hash, role)
        VALUES ($1, $2, $3, 'administrator')
        RETURNING id
        "#,
    )
    .bind(email)
    .bind("Admin")
    .bind(hash)
    .fetch_one(pool)
    .await
    .context("failed to insert bootstrap admin user")?;

    Ok(row.0)
}

/// Look up a user by email and verify their password. Returns `Ok(None)` for
/// both "no such user" and "wrong password" so callers cannot leak whether an
/// email exists.
pub async fn verify_password(pool: &PgPool, email: &str, password: &str) -> Result<Option<User>> {
    let row: Option<(Uuid, String, String, String, String, OffsetDateTime)> = sqlx::query_as(
        r#"
        SELECT id, email, name, password_hash, role, created_at
        FROM users
        WHERE email = $1
        "#,
    )
    .bind(email)
    .fetch_optional(pool)
    .await
    .context("failed to query user by email")?;

    let Some((id, email, name, password_hash, role, created_at)) = row else {
        // Run a dummy argon2 verify against a fixed hash to even out timing.
        // We ignore the result; this exists purely to mitigate trivial
        // "does this email exist?" timing oracles.
        let _ = verify_password_hash(password, DUMMY_HASH);
        return Ok(None);
    };

    if verify_password_hash(password, &password_hash)? {
        Ok(Some(User {
            id,
            email,
            name,
            role,
            created_at: offset_to_chrono(created_at),
        }))
    } else {
        Ok(None)
    }
}

/// Fetch a user by id. Returns `None` if not found.
pub async fn get_user(pool: &PgPool, id: Uuid) -> Result<Option<User>> {
    let row: Option<(Uuid, String, String, String, OffsetDateTime)> = sqlx::query_as(
        r#"
        SELECT id, email, name, role, created_at
        FROM users
        WHERE id = $1
        "#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .context("failed to query user by id")?;

    Ok(row.map(|(id, email, name, role, created_at)| User {
        id,
        email,
        name,
        role,
        created_at: offset_to_chrono(created_at),
    }))
}

/// Pre-computed argon2 hash of the literal string "dummy" used only to keep
/// timing of "user not found" roughly equivalent to "wrong password". The
/// value is never used as a real credential.
const DUMMY_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c29tZXNhbHRzb21lc2FsdA$\
    Fx+sPPfCW4s7tqs0nF5MfvSaU/eOD3NJxd3bbK2aPwM";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_and_verify_roundtrip() {
        let h = hash_password("hunter2").unwrap();
        assert!(verify_password_hash("hunter2", &h).unwrap());
        assert!(!verify_password_hash("wrong", &h).unwrap());
    }

    #[test]
    fn dummy_hash_parses() {
        // The internal dummy hash should parse so the timing-even path works.
        let r = verify_password_hash("anything", DUMMY_HASH);
        assert!(r.is_ok(), "dummy hash must be parseable");
        assert!(
            !r.unwrap(),
            "dummy hash should not verify against 'anything'"
        );
    }

    #[test]
    fn hash_is_salted() {
        let a = hash_password("same").unwrap();
        let b = hash_password("same").unwrap();
        assert_ne!(a, b, "argon2 hashes should include a random salt");
    }
}
