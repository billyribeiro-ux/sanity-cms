use std::sync::Arc;

use content_lake_core::events::bus::{EventBus, PresenceBus};
use content_lake_core::schema::SchemaRegistry;
use sqlx::PgPool;
use uuid::Uuid;

use crate::config::AppConfig;

/// Shared application state, passed to all handlers via Axum's `State` extractor.
/// Wrapped in `Arc` so cloning is cheap.
#[derive(Clone)]
pub struct AppState {
    inner: Arc<InnerState>,
}

#[allow(dead_code)]
struct InnerState {
    pub pool: PgPool,
    pub config: AppConfig,
    pub event_bus: EventBus,
    pub presence_bus: PresenceBus,
    pub schema_registry: Arc<SchemaRegistry>,
    pub node_id: Uuid,
}

impl AppState {
    pub fn new(
        pool: PgPool,
        config: AppConfig,
        event_bus: EventBus,
        presence_bus: PresenceBus,
        schema_registry: Arc<SchemaRegistry>,
        node_id: Uuid,
    ) -> Self {
        Self {
            inner: Arc::new(InnerState {
                pool,
                config,
                event_bus,
                presence_bus,
                schema_registry,
                node_id,
            }),
        }
    }

    pub fn pool(&self) -> &PgPool {
        &self.inner.pool
    }

    pub fn config(&self) -> &AppConfig {
        &self.inner.config
    }

    pub fn event_bus(&self) -> &EventBus {
        &self.inner.event_bus
    }

    pub fn presence_bus(&self) -> &PresenceBus {
        &self.inner.presence_bus
    }

    pub fn schema_registry(&self) -> &SchemaRegistry {
        &self.inner.schema_registry
    }

    /// Cluster-local node identifier, generated once at startup and stamped
    /// into every outgoing event so remote nodes can dedupe echoes received
    /// over Postgres `LISTEN/NOTIFY`.
    pub fn node_id(&self) -> Uuid {
        self.inner.node_id
    }
}
