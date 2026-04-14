pub mod assets;
pub mod auth;
pub mod doc;
pub mod health;
pub mod listen;
pub mod mutate;
pub mod projects;
pub mod query;

use axum::Router;

use crate::state::AppState;

/// Assemble the full router with all route groups.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .merge(health::routes())
        .merge(auth::routes())
        .merge(projects::routes())
        .merge(doc::routes())
        .merge(mutate::routes())
        .merge(listen::routes())
        .merge(assets::routes())
        .merge(query::routes())
        // Future: .merge(presence::routes())
        .with_state(state)
}
