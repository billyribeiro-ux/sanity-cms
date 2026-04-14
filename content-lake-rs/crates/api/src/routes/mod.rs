pub mod health;

use axum::Router;

use crate::state::AppState;

/// Assemble the full router with all route groups.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .merge(health::routes())
        // Future: .merge(query::routes())
        // Future: .merge(mutate::routes())
        // Future: .merge(doc::routes())
        // Future: .merge(listen::routes())
        // Future: .merge(auth::routes())
        // Future: .merge(assets::routes())
        // Future: .merge(presence::routes())
        .with_state(state)
}
