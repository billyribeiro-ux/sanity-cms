# Content Lake RS

A self-hosted, Sanity-compatible Content Lake API built in Rust.

## Stack

- **Axum 0.8** — HTTP framework with Tower middleware
- **Tokio** — Async runtime
- **SQLx 0.8** — Async PostgreSQL with compile-time query checking
- **PostgreSQL** — Document storage via JSONB

## Project Structure

```
content-lake-rs/
├── crates/
│   ├── api/        # Axum HTTP server (binary)
│   ├── core/       # Business logic (library)
│   └── groq/       # GROQ parser + evaluator (library)
├── migrations/     # SQLx database migrations
├── docker-compose.yml
└── Dockerfile
```

## Quick Start

### Prerequisites

- Rust 1.75+
- PostgreSQL 15+ (or Docker)

### 1. Start PostgreSQL

```bash
docker compose up -d postgres
```

### 2. Configure Environment

```bash
cp .env.example .env
# Edit .env if needed (defaults work with docker-compose)
```

### 3. Run

```bash
cargo run --bin content-lake-api
```

The server starts on `http://localhost:3030`.

### 4. Verify

```bash
# Lightweight ping (no DB)
curl http://localhost:3030/v1/ping

# Full health check (verifies DB connection)
curl http://localhost:3030/health
```

## Development

```bash
# Check compilation
cargo check

# Run tests
cargo test

# Run with Docker Compose (Postgres + API)
docker compose up
```

## API Routes (Planned)

| Method | Path | Status |
|--------|------|--------|
| `GET` | `/health` | ✅ Phase 0 |
| `GET` | `/v1/ping` | ✅ Phase 0 |
| `GET` | `/v1/data/query/{dataset}` | Phase 2 |
| `POST` | `/v1/data/mutate/{dataset}` | Phase 1 |
| `GET` | `/v1/data/doc/{dataset}/{id}` | Phase 1 |
| `GET` | `/v1/data/listen/{dataset}` | Phase 3 |
| `POST` | `/v1/assets/images/{dataset}` | Phase 5 |
| `WS` | `/v1/presence/{dataset}` | Phase 6 |

## Architecture

See [RUST_API_PLAN.md](../sanity-main/RUST_API_PLAN.md) for the full implementation plan.
