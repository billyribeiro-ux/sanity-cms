use std::env;

/// Application configuration loaded from environment variables.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct AppConfig {
    /// Server host to bind to.
    pub host: String,
    /// Server port to bind to.
    pub port: u16,
    /// PostgreSQL connection URL.
    pub database_url: String,
    /// Maximum database connections in the pool.
    pub db_max_connections: u32,
    /// Minimum database connections in the pool.
    pub db_min_connections: u32,
    /// JWT signing secret.
    pub jwt_secret: String,
    /// Event bus channel capacity.
    pub event_bus_capacity: usize,
    /// Log level (e.g., "info", "debug", "trace").
    pub log_level: String,
}

impl AppConfig {
    /// Load configuration from environment variables with sensible defaults.
    pub fn from_env() -> Result<Self, env::VarError> {
        Ok(Self {
            host: env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string()),
            port: env::var("PORT")
                .unwrap_or_else(|_| "3030".to_string())
                .parse()
                .expect("PORT must be a valid u16"),
            database_url: env::var("DATABASE_URL")?,
            db_max_connections: env::var("DB_MAX_CONNECTIONS")
                .unwrap_or_else(|_| "20".to_string())
                .parse()
                .expect("DB_MAX_CONNECTIONS must be a valid u32"),
            db_min_connections: env::var("DB_MIN_CONNECTIONS")
                .unwrap_or_else(|_| "5".to_string())
                .parse()
                .expect("DB_MIN_CONNECTIONS must be a valid u32"),
            jwt_secret: env::var("JWT_SECRET")
                .unwrap_or_else(|_| "dev-secret-change-me-in-production".to_string()),
            event_bus_capacity: env::var("EVENT_BUS_CAPACITY")
                .unwrap_or_else(|_| "1024".to_string())
                .parse()
                .expect("EVENT_BUS_CAPACITY must be a valid usize"),
            log_level: env::var("LOG_LEVEL").unwrap_or_else(|_| "info".to_string()),
        })
    }

    /// Build the socket address string.
    pub fn addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}
