use tower_http::cors::{Any, CorsLayer};

/// Build the CORS layer. Permissive for development; tighten for production.
pub fn cors_layer() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any)
}
