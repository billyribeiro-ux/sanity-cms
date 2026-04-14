//! Library crate for the content-lake HTTP API.
//!
//! Exposes the same modules the binary uses so integration tests can build
//! an in-process app against an ephemeral Postgres, without shelling out to
//! a separate `cargo run`.

pub mod config;
pub mod error;
pub mod middleware;
pub mod routes;
pub mod state;
