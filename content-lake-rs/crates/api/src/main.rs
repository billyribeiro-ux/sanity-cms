mod config;
mod error;
mod middleware;
mod routes;
mod state;

use content_lake_core::events::bus::EventBus;
use sqlx::postgres::PgPoolOptions;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load .env file if present (dev convenience)
    let _ = dotenvy::dotenv();

    // Load configuration
    let config = config::AppConfig::from_env()
        .map_err(|e| anyhow::anyhow!("Failed to load config: {e}. Is DATABASE_URL set?"))?;

    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&config.log_level)),
        )
        .json()
        .init();

    tracing::info!("Starting Content Lake API server");

    // Create database connection pool
    let pool = PgPoolOptions::new()
        .max_connections(config.db_max_connections)
        .min_connections(config.db_min_connections)
        .connect(&config.database_url)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to connect to database: {e}"))?;

    tracing::info!("Connected to PostgreSQL");

    // Run migrations
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to run migrations: {e}"))?;

    tracing::info!("Database migrations applied");

    // Bootstrap the default project + dataset so Studio can connect
    // without requiring a manual provisioning step.
    let (project_id, dataset_id) = content_lake_core::document::repo::bootstrap(
        &pool,
        &config.bootstrap_project,
        &config.bootstrap_dataset,
    )
    .await
    .map_err(|e| anyhow::anyhow!("bootstrap failed: {e}"))?;
    tracing::info!(
        project = %config.bootstrap_project,
        dataset = %config.bootstrap_dataset,
        project_id = %project_id,
        dataset_id = %dataset_id,
        "Bootstrap project/dataset ready"
    );

    // Bootstrap an admin user on first startup. If the users table already
    // has rows, this is a no-op.
    let admin_id =
        content_lake_core::auth::ensure_admin(&pool, &config.admin_email, &config.admin_password)
            .await
            .map_err(|e| anyhow::anyhow!("admin bootstrap failed: {e}"))?;
    tracing::info!(admin_id = %admin_id, email = %config.admin_email, "Admin user ready");
    if config.admin_password == "admin" {
        tracing::warn!(
            "Using default admin password \u{2014} set ADMIN_PASSWORD in production"
        );
    }
    if config.auth_disabled {
        tracing::warn!("AUTH_DISABLED=1 \u{2014} skipping JWT verification on all requests");
    }

    // Create event bus
    let event_bus = EventBus::new(config.event_bus_capacity);

    // Load the optional document schema registry (SCHEMA_FILE env).
    let schema_registry = std::sync::Arc::new(
        content_lake_core::schema::SchemaRegistry::from_env_or_empty(),
    );
    if schema_registry.is_empty() {
        tracing::info!("Schema validation disabled (no SCHEMA_FILE or empty registry)");
    } else {
        tracing::info!(types = schema_registry.len(), "Schema validation enabled");
    }

    // Build application state
    let state = state::AppState::new(pool, config.clone(), event_bus, schema_registry);

    // Build router with middleware
    let app = routes::build_router(state.clone())
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            middleware::auth::auth_middleware,
        ))
        .layer(middleware::request_tracing::trace_layer())
        .layer(middleware::cors::cors_layer());

    // Start server
    let addr = config.addr();
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("Listening on {addr}");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    tracing::info!("Server shut down gracefully");
    Ok(())
}

/// Wait for SIGINT (Ctrl+C) or SIGTERM for graceful shutdown.
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => { tracing::info!("Received Ctrl+C, shutting down..."); }
        _ = terminate => { tracing::info!("Received SIGTERM, shutting down..."); }
    }
}
