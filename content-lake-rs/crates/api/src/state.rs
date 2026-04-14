use std::sync::Arc;

use content_lake_core::events::bus::{EventBus, PresenceBus};
use sqlx::PgPool;

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
}

impl AppState {
    pub fn new(pool: PgPool, config: AppConfig, event_bus: EventBus) -> Self {
        let presence_bus = PresenceBus::new(1024);
        Self {
            inner: Arc::new(InnerState {
                pool,
                config,
                event_bus,
                presence_bus,
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
}
