# Rust Content Lake API — Principal Engineer L7 Implementation Plan

> Build a self-hosted, Sanity-compatible Content Lake API in Rust (Axum + Tokio + SQLx + PostgreSQL).

---

## 1. Executive Summary

A Rust API server that speaks the exact HTTP contract Sanity Studio expects from the Content Lake backend. Stores documents in PostgreSQL with JSONB. Implements GROQ query execution, real-time SSE listeners, the draft/published/version document model, presence, permissions, and asset management.

Designed for horizontal scalability, sub-10ms p99 reads, and exactly-once mutation delivery.

---

## 2. Contracts Extracted from Sanity Studio

### 2.1 Document Model

From `@sanity/types/src/documents/types.ts`:

```rust
struct SanityDocument {
    _id: String,        // globally unique, prefixed for drafts/versions
    _type: String,      // schema type name
    _createdAt: String, // ISO 8601
    _updatedAt: String, // ISO 8601
    _rev: String,       // revision ID (changes on every mutation)
    // ... arbitrary JSON fields stored as serde_json::Value
}
```

**ID conventions** (from Studio utilities):
- Published: `{id}`
- Draft: `drafts.{id}`
- Version: `versions.{releaseId}.{id}`

### 2.2 Mutation Operations

From `@sanity/types/src/mutations/types.ts`:

| Operation | Semantics |
|-----------|-----------|
| `create` | Insert new doc. Fails if `_id` exists. Server can generate `_id`. |
| `createOrReplace` | Upsert. Requires `_id`. |
| `createIfNotExists` | Insert only if `_id` absent. No-op otherwise. |
| `delete` | Remove by `{id}` or `{query, params}`. |
| `patch` | Partial update with `set`, `setIfMissing`, `unset`, `inc`, `dec`, `insert`, `diffMatchPatch`. Supports `ifRevisionID` for optimistic concurrency. |

**Patch sub-operations** (from `PatchOperations`):

| Field | Type | Purpose |
|-------|------|---------|
| `set` | `{path: value}` | Set fields |
| `setIfMissing` | `{path: value}` | Set only if field absent |
| `merge` | `{path: value}` | Deep merge objects |
| `unset` | `[path]` | Remove fields |
| `inc` / `dec` | `{path: number}` | Atomic increment/decrement |
| `insert` | `{before\|after\|replace: path, items}` | Array manipulation |
| `diffMatchPatch` | `{path: patch_string}` | Text diff patching |
| `ifRevisionID` | `string` | Optimistic concurrency guard |

### 2.3 Mutation Response

```rust
struct MutationResult {
    transaction_id: String,
    document_id: String,    // or document_ids: Vec<String>
    results: Vec<IdResult>,
}
struct IdResult { id: String }
```

### 2.4 Real-Time Listener Protocol

From `getPairListener.ts` — Studio calls `client.observable.listen()`:

**Request**: `GET /vX/data/listen/{dataset}?query=...&includeResult=false&effectFormat=mendoza`

**SSE event types the Studio expects**:
- `welcome` — connection established
- `mutation` — document changed (with `documentId`, `previousRev`, `resultRev`, `transactionId`, `effects` in mendoza format, `timestamp`)
- `reconnect` — server signals client should reconnect

**Critical behaviors**:
- Studio tracks `previousRev` → `resultRev` chains for gap detection
- On gap: Studio refetches full snapshots and rebases local state
- `transactionTotalEvents` / `transactionCurrentEvent` for multi-doc transactions
- Studio reconnects with snapshot refetch after 20s disconnect

### 2.5 Authentication

From `authStore/types.ts`:
- Token-based auth (`Authorization: Bearer {token}`)
- Auth state: `{ authenticated: boolean, currentUser, client }`
- Login flow is browser-based (OAuth providers)

### 2.6 Permissions / Grants

From `documentPairPermissions.ts`:
- `checkDocumentPermission(action, document)` where action = `create | update | delete`
- Document-level permission checks against grants
- Grants are filter-based (GROQ filter expressions)

### 2.7 Presence

From `presence-store.ts`:
- Bifur WebSocket transport for real-time presence
- Message types: `state` (locations), `rollCall`, `disconnect`
- Sessions identified by `sessionId` + `userId`
- Location = `{ documentId, path, lastActiveAt }`

---

## 3. Architecture Overview

```
                    ┌─────────────────────────────────────┐
                    │          Load Balancer / TLS         │
                    └──────────────┬──────────────────────┘
                                   │
                    ┌──────────────▼──────────────────────┐
                    │         Axum HTTP Server             │
                    │  ┌───────┐ ┌───────┐ ┌───────────┐  │
                    │  │ Query │ │Mutate │ │ Listen/SSE│  │
                    │  │Routes │ │Routes │ │  Routes   │  │
                    │  └───┬───┘ └───┬───┘ └─────┬─────┘  │
                    │      │         │           │         │
                    │  ┌───▼─────────▼───────────▼─────┐  │
                    │  │      Core Service Layer        │  │
                    │  │  ┌─────────┐ ┌──────────────┐  │  │
                    │  │  │  GROQ   │ │  Mutation    │  │  │
                    │  │  │ Engine  │ │  Executor    │  │  │
                    │  │  └─────────┘ └──────────────┘  │  │
                    │  │  ┌─────────┐ ┌──────────────┐  │  │
                    │  │  │  Auth   │ │  Permissions │  │  │
                    │  │  │ Service │ │  Engine      │  │  │
                    │  │  └─────────┘ └──────────────┘  │  │
                    │  └───────────────┬────────────────┘  │
                    │                  │                    │
                    │  ┌───────────────▼────────────────┐  │
                    │  │     Event Bus (tokio broadcast) │  │
                    │  └───────────────┬────────────────┘  │
                    └──────────────────┼───────────────────┘
                                       │
                    ┌──────────────────▼───────────────────┐
                    │          PostgreSQL (SQLx)            │
                    │  ┌──────────┐ ┌───────────────────┐  │
                    │  │documents │ │  transactions      │  │
                    │  │ (JSONB)  │ │  (event sourcing)  │  │
                    │  └──────────┘ └───────────────────┘  │
                    │  ┌──────────┐ ┌───────────────────┐  │
                    │  │ assets   │ │  projects/users    │  │
                    │  └──────────┘ └───────────────────┘  │
                    └──────────────────────────────────────┘
```

---

## 4. Technology Choices & Crate Selection

| Concern | Crate | Rationale |
|---------|-------|-----------|
| HTTP framework | `axum 0.8` | Tower middleware, extractors, SSE native support |
| Async runtime | `tokio` | Industry standard, work-stealing scheduler |
| Database | `sqlx 0.8` (Postgres) | Compile-time query checking, async, connection pooling |
| Serialization | `serde` + `serde_json` | Zero-cost JSON ↔ Rust, JSONB compat |
| Auth / JWT | `jsonwebtoken` | Token verification |
| Password hashing | `argon2` | For local user accounts |
| SSE | `axum::response::Sse` | Native Axum support |
| WebSocket | `axum::extract::ws` | For presence transport |
| GROQ parser | Custom (`pest` or `nom`) | No existing Rust GROQ crate |
| Text diffing | `similar` | For `diffMatchPatch` operations |
| UUID | `uuid` | Transaction IDs, revision IDs |
| Tracing | `tracing` + `tracing-subscriber` | Structured logging |
| Config | `config` + `dotenvy` | Environment-based config |
| Migrations | `sqlx-cli` | Schema migrations |
| Testing | `tokio::test` + `sqlx::test` | Async test harness with test databases |
| Error handling | `thiserror` + `anyhow` | Typed errors for API, anyhow for internal |

---

## 5. Database Schema

### 5.1 Core Tables

```sql
-- Projects (multi-tenant isolation)
CREATE TABLE projects (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Datasets (each project has multiple datasets)
CREATE TABLE datasets (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id UUID NOT NULL REFERENCES projects(id),
    name TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(project_id, name)
);

-- Documents (the core entity)
CREATE TABLE documents (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    dataset_id UUID NOT NULL REFERENCES datasets(id),
    document_id TEXT NOT NULL,          -- Sanity _id (e.g., "drafts.abc123")
    doc_type TEXT NOT NULL,             -- Sanity _type
    revision TEXT NOT NULL,             -- _rev (changes on every mutation)
    content JSONB NOT NULL DEFAULT '{}',-- Full document body
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted BOOLEAN NOT NULL DEFAULT false,
    UNIQUE(dataset_id, document_id)
);

CREATE INDEX idx_documents_type ON documents(dataset_id, doc_type);
CREATE INDEX idx_documents_content ON documents USING GIN(content jsonb_path_ops);
CREATE INDEX idx_documents_updated ON documents(dataset_id, updated_at DESC);

-- Transaction log (event sourcing / history)
CREATE TABLE transactions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    dataset_id UUID NOT NULL REFERENCES datasets(id),
    transaction_id TEXT NOT NULL UNIQUE,
    author TEXT,                         -- user ID
    mutations JSONB NOT NULL,            -- array of mutation operations
    effects JSONB,                       -- mendoza effects for each doc
    timestamp TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_transactions_dataset ON transactions(dataset_id, timestamp DESC);
CREATE INDEX idx_transactions_tid ON transactions(transaction_id);

-- Transaction-Document junction (which docs a txn touched)
CREATE TABLE transaction_documents (
    transaction_id UUID NOT NULL REFERENCES transactions(id),
    document_id TEXT NOT NULL,
    previous_rev TEXT,
    result_rev TEXT,
    PRIMARY KEY(transaction_id, document_id)
);
```

### 5.2 Auth & Permissions Tables

```sql
CREATE TABLE users (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    email TEXT NOT NULL UNIQUE,
    display_name TEXT,
    password_hash TEXT,                 -- NULL for OAuth-only users
    provider TEXT,                      -- 'local', 'google', 'github', etc.
    provider_id TEXT,
    avatar_url TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE api_tokens (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id UUID NOT NULL REFERENCES projects(id),
    user_id UUID REFERENCES users(id), -- NULL for robot tokens
    token_hash TEXT NOT NULL UNIQUE,
    label TEXT NOT NULL,
    role TEXT NOT NULL DEFAULT 'editor', -- 'admin', 'editor', 'viewer', 'custom'
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ
);

-- GROQ-filter-based grants (mirrors Sanity's grant system)
CREATE TABLE grants (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id UUID NOT NULL REFERENCES projects(id),
    role TEXT NOT NULL,
    dataset_pattern TEXT NOT NULL DEFAULT '*',
    filter TEXT NOT NULL DEFAULT 'true', -- GROQ filter expression
    permissions TEXT[] NOT NULL,          -- {'create','read','update','delete'}
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

### 5.3 Assets Table

```sql
CREATE TABLE assets (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    dataset_id UUID NOT NULL REFERENCES datasets(id),
    asset_id TEXT NOT NULL,              -- sanity asset ID
    asset_type TEXT NOT NULL,            -- 'image' or 'file'
    path TEXT NOT NULL,                  -- storage path
    filename TEXT NOT NULL,
    mime_type TEXT NOT NULL,
    size_bytes BIGINT NOT NULL,
    metadata JSONB DEFAULT '{}',         -- dimensions, palette, etc.
    sha256 TEXT NOT NULL,
    uploaded_by UUID REFERENCES users(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(dataset_id, asset_id)
);
```

### 5.4 Presence Table (optional persistence)

```sql
CREATE TABLE presence_sessions (
    session_id TEXT PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id),
    project_id UUID NOT NULL REFERENCES projects(id),
    locations JSONB NOT NULL DEFAULT '[]',
    last_seen TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_presence_project ON presence_sessions(project_id, last_seen);
```

---

## 6. Rust Project Structure

```
content-lake-rs/
├── Cargo.toml                    # Workspace root
├── Cargo.lock
├── .env.example
├── migrations/                   # SQLx migrations
│   ├── 001_initial_schema.sql
│   ├── 002_auth_tables.sql
│   └── 003_assets.sql
├── crates/
│   ├── api/                      # Axum HTTP server (binary)
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── main.rs           # Entry point, server bootstrap
│   │       ├── config.rs         # Configuration loading
│   │       ├── state.rs          # AppState (pool, services)
│   │       ├── error.rs          # API error types → JSON responses
│   │       ├── middleware/
│   │       │   ├── mod.rs
│   │       │   ├── auth.rs       # Bearer token extraction + validation
│   │       │   ├── cors.rs       # CORS configuration
│   │       │   ├── project.rs    # Project/dataset resolution from URL
│   │       │   └── tracing.rs    # Request tracing spans
│   │       ├── routes/
│   │       │   ├── mod.rs        # Router assembly
│   │       │   ├── query.rs      # GET  /vX/data/query/{dataset}
│   │       │   ├── mutate.rs     # POST /vX/data/mutate/{dataset}
│   │       │   ├── doc.rs        # GET  /vX/data/doc/{dataset}/{id}
│   │       │   ├── listen.rs     # GET  /vX/data/listen/{dataset} (SSE)
│   │       │   ├── history.rs    # GET  /vX/data/history/{dataset}
│   │       │   ├── assets.rs     # POST /vX/assets/images/{dataset}
│   │       │   ├── auth.rs       # POST /vX/auth/login, /logout, /providers
│   │       │   ├── users.rs      # GET  /vX/users/me
│   │       │   ├── projects.rs   # GET  /vX/projects/{projectId}
│   │       │   └── presence.rs   # WebSocket /vX/presence/{dataset}
│   │       └── extractors/
│   │           ├── mod.rs
│   │           ├── auth.rs       # CurrentUser extractor
│   │           └── dataset.rs    # Dataset extractor
│   │
│   ├── core/                     # Business logic (library)
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── document/
│   │       │   ├── mod.rs
│   │       │   ├── model.rs      # SanityDocument, IdPair, revision logic
│   │       │   ├── id.rs         # Draft/published/version ID parsing
│   │       │   └── validate.rs   # Document validation
│   │       ├── mutation/
│   │       │   ├── mod.rs
│   │       │   ├── types.rs      # Mutation, PatchOperations enums
│   │       │   ├── executor.rs   # Apply mutations to documents
│   │       │   ├── patch.rs      # Patch operations (set, unset, inc, etc.)
│   │       │   └── diff_match.rs # diffMatchPatch implementation
│   │       ├── query/
│   │       │   ├── mod.rs
│   │       │   ├── parser.rs     # GROQ → AST (pest/nom)
│   │       │   ├── ast.rs        # GROQ AST types
│   │       │   ├── evaluator.rs  # AST → SQL or in-memory eval
│   │       │   ├── functions.rs  # GROQ built-in functions
│   │       │   └── sql.rs        # GROQ → PostgreSQL query transpiler
│   │       ├── auth/
│   │       │   ├── mod.rs
│   │       │   ├── token.rs      # JWT/token creation + validation
│   │       │   ├── user.rs       # User service
│   │       │   └── grants.rs     # Permission grant evaluation
│   │       ├── events/
│   │       │   ├── mod.rs
│   │       │   ├── bus.rs        # In-process event bus (tokio::broadcast)
│   │       │   ├── types.rs      # MutationEvent, WelcomeEvent, etc.
│   │       │   └── listener.rs   # Subscription management
│   │       ├── presence/
│   │       │   ├── mod.rs
│   │       │   ├── store.rs      # In-memory presence state
│   │       │   └── transport.rs  # WebSocket presence messages
│   │       ├── assets/
│   │       │   ├── mod.rs
│   │       │   ├── upload.rs     # File upload processing
│   │       │   ├── storage.rs    # Local FS or S3 abstraction
│   │       │   └── metadata.rs   # Image metadata extraction
│   │       └── history/
│   │           ├── mod.rs
│   │           └── transaction_log.rs  # Transaction history queries
│   │
│   └── groq/                     # Standalone GROQ parser + evaluator
│       ├── Cargo.toml
│       └── src/
│           ├── lib.rs
│           ├── lexer.rs          # Tokenizer
│           ├── parser.rs         # Recursive descent or PEG parser
│           ├── ast.rs            # Full GROQ AST
│           ├── eval.rs           # In-memory evaluator (for grants/filters)
│           ├── sql_gen.rs        # GROQ → SQL transpilation
│           ├── functions.rs      # count(), defined(), references(), etc.
│           └── tests/
│               ├── parse_tests.rs
│               ├── eval_tests.rs
│               └── sql_tests.rs
│
├── tests/                        # Integration tests
│   ├── api_tests.rs              # Full HTTP request/response tests
│   ├── mutation_tests.rs         # Mutation semantics
│   ├── listener_tests.rs         # SSE listener contract
│   ├── auth_tests.rs             # Authentication flows
│   └── groq_tests.rs             # GROQ query correctness
│
└── benches/                      # Benchmarks
    ├── query_bench.rs
    └── mutation_bench.rs
```

---

## 7. API Routes (Sanity-Compatible)

### 7.1 Data API

| Method | Path | Handler | Description |
|--------|------|---------|-------------|
| `GET` | `/v1/data/query/{dataset}?query={groq}` | `query::handle_query` | Execute GROQ query |
| `POST` | `/v1/data/mutate/{dataset}` | `mutate::handle_mutate` | Execute mutations in a transaction |
| `GET` | `/v1/data/doc/{dataset}/{id+}` | `doc::handle_get_doc` | Fetch document(s) by ID |
| `GET` | `/v1/data/listen/{dataset}?query={groq}` | `listen::handle_listen` | SSE real-time listener |
| `GET` | `/v1/data/history/{dataset}/transactions` | `history::handle_history` | Transaction log |

### 7.2 Assets API

| Method | Path | Handler | Description |
|--------|------|---------|-------------|
| `POST` | `/v1/assets/images/{dataset}` | `assets::upload_image` | Upload image |
| `POST` | `/v1/assets/files/{dataset}` | `assets::upload_file` | Upload file |
| `GET` | `/cdn.sanity.io/images/{project}/{dataset}/{id}` | `assets::serve` | Serve asset |

### 7.3 Auth API

| Method | Path | Handler | Description |
|--------|------|---------|-------------|
| `POST` | `/v1/auth/login` | `auth::login` | Email/password login |
| `POST` | `/v1/auth/logout` | `auth::logout` | Invalidate token |
| `GET` | `/v1/auth/providers` | `auth::providers` | List OAuth providers |
| `GET` | `/v1/auth/callback/{provider}` | `auth::oauth_callback` | OAuth callback |
| `GET` | `/v1/users/me` | `users::me` | Current user profile |

### 7.4 Project API

| Method | Path | Handler | Description |
|--------|------|---------|-------------|
| `GET` | `/v1/projects/{projectId}` | `projects::get` | Project metadata |
| `GET` | `/v1/projects/{projectId}/datasets` | `projects::datasets` | List datasets |

### 7.5 Presence API

| Method | Path | Handler | Description |
|--------|------|---------|-------------|
| `WS` | `/v1/presence/{dataset}` | `presence::ws_handler` | WebSocket presence |

---

## 8. Implementation Phases

### Phase 0: Foundation (Week 1-2)

**Deliverables**: Bootable server, database connection, health check, CI pipeline.

```
Tasks:
├── Cargo workspace setup (api, core, groq crates)
├── PostgreSQL connection via SQLx with connection pooling
├── Database migrations (001_initial_schema)
├── AppState struct (PgPool, config, event bus)
├── Axum router skeleton with health check endpoint
├── Configuration loading (env vars, .env file)
├── Error type → JSON response mapping
├── Tracing/logging setup
├── Docker Compose (Postgres, server)
├── CI: cargo check, cargo test, cargo clippy, cargo fmt
└── Integration test harness with test database
```

**Key design decisions**:
- `AppState` is `Arc<InnerState>` passed via Axum's `State` extractor
- Connection pool: `sqlx::PgPool` with `max_connections=20`, `min_connections=5`
- All timestamps in UTC, stored as `TIMESTAMPTZ`

### Phase 1: Document CRUD + Mutations (Week 3-5)

**Deliverables**: Full mutation pipeline, document retrieval, transaction log.

```
Tasks:
├── Document model (SanityDocument ↔ JSONB serialization)
├── Document ID parsing (published, draft, version)
├── Revision generation (UUID v7 for time-ordering)
├── Mutation type definitions (Rust enums matching Sanity's types)
├── Mutation executor:
│   ├── create → INSERT with conflict check
│   ├── createOrReplace → INSERT ON CONFLICT UPDATE
│   ├── createIfNotExists → INSERT ON CONFLICT DO NOTHING
│   ├── delete → UPDATE SET deleted=true (soft delete)
│   └── patch → Fetch, apply in-memory, UPDATE
├── Patch operations:
│   ├── set / setIfMissing → JSONB path set
│   ├── unset → JSONB path remove
│   ├── inc / dec → Atomic numeric update
│   ├── insert (before/after/replace) → Array manipulation
│   ├── merge → Deep JSONB merge
│   └── diffMatchPatch → Text diff application
├── ifRevisionID optimistic concurrency (WHERE rev = $expected)
├── Transaction wrapping (all mutations in one TX)
├── Transaction log recording
├── POST /v1/data/mutate/{dataset} route
├── GET /v1/data/doc/{dataset}/{ids} route
├── _createdAt / _updatedAt / _rev auto-management
└── Comprehensive mutation tests (port Sanity's test cases)
```

**Critical implementation detail for patches**:
```rust
// Patch application is done in-memory, not pure SQL,
// because diffMatchPatch and complex array inserts require it.
// Flow: BEGIN TX → SELECT doc FOR UPDATE → apply patches → UPDATE doc → COMMIT
async fn execute_patch(pool: &PgPool, patch: PatchMutation) -> Result<MutationResult> {
    let mut tx = pool.begin().await?;
    let doc = sqlx::query_as!(...)
        .fetch_optional(&mut *tx).await?;
    let patched = apply_patch_operations(doc, &patch.operations)?;
    sqlx::query!("UPDATE documents SET content = $1, revision = $2 ...")
        .execute(&mut *tx).await?;
    tx.commit().await?;
}
```

### Phase 2: GROQ Engine (Week 6-9)

**Deliverables**: GROQ parser, SQL transpiler, in-memory evaluator.

This is the hardest phase. GROQ is a full query language with pipes, projections, filters, ordering, slicing, joins, and functions.

```
Tasks:
├── GROQ Lexer (tokenizer)
│   ├── Identifiers, strings, numbers, booleans, null
│   ├── Operators: ==, !=, <, >, <=, >=, &&, ||, !, in, match
│   ├── Punctuation: ., ->, [], {}, (), |, ..
│   └── Keywords: true, false, null, order, asc, desc
├── GROQ Parser (recursive descent)
│   ├── *[filter] — dataset scan with filter
│   ├── *[filter]{projection} — field projection
│   ├── *[filter] | order(field asc) — pipe + ordering
│   ├── *[filter][0..10] — slicing
│   ├── @, ^ — current/parent document references
│   ├── references() — reference resolution
│   ├── count(), defined(), coalesce(), select()
│   ├── Nested projections with dereference (->)
│   └── String functions: upper(), lower(), etc.
├── GROQ → SQL Transpilation Strategy:
│   ├── *[_type == "post"] → SELECT content FROM documents
│   │   WHERE content->>'_type' = 'post' AND NOT deleted
│   ├── Projections → Build JSON in SQL or post-process
│   ├── Filters → WHERE clauses on JSONB paths
│   ├── order() → ORDER BY jsonb path extraction
│   ├── [0..10] → LIMIT/OFFSET
│   ├── count() → SELECT COUNT(*)
│   └── references() → JSONB containment @> operator
├── In-Memory Evaluator (for grants filter evaluation)
│   └── Evaluate GROQ filter against a single document in memory
├── GET /v1/data/query/{dataset} route
├── Query parameter handling ($params)
└── GROQ test suite (200+ test cases)
```

**Transpilation strategy** — two modes:

1. **SQL mode** (default): Transpile GROQ → PostgreSQL query. Handles 90% of queries. Uses JSONB operators (`->>`, `@>`, `jsonb_path_query`).

2. **Hybrid mode**: For complex projections/dereferences, fetch candidate rows via SQL filter, then apply projection + joins in-memory in Rust.

```rust
// Example GROQ → SQL
// GROQ: *[_type == "post" && author._ref == $authorId] | order(publishedAt desc) [0..10]
// SQL:
// SELECT content FROM documents
// WHERE dataset_id = $1
//   AND NOT deleted
//   AND content->>'_type' = 'post'
//   AND content->'author'->>'_ref' = $2
// ORDER BY content->>'publishedAt' DESC
// LIMIT 10 OFFSET 0
```

### Phase 3: Real-Time Listeners (Week 10-11)

**Deliverables**: SSE listener endpoint, event bus, mutation broadcasting.

```
Tasks:
├── Event Bus (tokio::broadcast channel)
│   ├── MutationEvent type (matching Studio expectations)
│   ├── Per-dataset channels
│   └── Backpressure handling (lagged receivers)
├── Mutation pipeline integration:
│   └── After successful commit → broadcast MutationEvent to bus
├── SSE Listener endpoint:
│   ├── Parse listener query (GROQ filter for which docs to watch)
│   ├── Send 'welcome' event on connect
│   ├── Subscribe to event bus
│   ├── Filter events against listener query
│   ├── Format SSE events (id, event type, data JSON)
│   └── Send 'reconnect' on server-side issues
├── Mendoza effects generation:
│   ├── Compute diff between previous and current document
│   └── Encode as mendoza effect format
├── Multi-document transaction events:
│   ├── transactionTotalEvents / transactionCurrentEvent counters
│   └── Ordered delivery per transaction
├── Connection lifecycle:
│   ├── Heartbeat / keep-alive (every 30s)
│   ├── Client disconnect detection
│   └── Graceful shutdown propagation
└── Listener integration tests
```

**Event bus design for horizontal scaling** (future):
```
Phase 3a (single node): tokio::broadcast
Phase 3b (multi node):  PostgreSQL LISTEN/NOTIFY or Redis Streams
```

### Phase 4: Authentication & Authorization (Week 12-13)

**Deliverables**: Token auth, user management, grant-based permissions.

```
Tasks:
├── Token Service:
│   ├── JWT creation (HS256 or RS256)
│   ├── Token validation middleware
│   ├── API token (long-lived, stored hashed)
│   └── Session token (short-lived JWT)
├── Auth middleware (Axum layer):
│   ├── Extract Authorization header
│   ├── Validate token → CurrentUser
│   ├── Inject user into request extensions
│   └── Public route bypass (configurable)
├── User management:
│   ├── Local email/password (Argon2 hashing)
│   ├── OAuth2 flow (Google, GitHub)
│   ├── User profile endpoints
│   └── GET /v1/users/me
├── Grant-based permissions:
│   ├── Load grants for user's role
│   ├── Evaluate GROQ filter grants against target document
│   ├── Permission check: create, read, update, delete
│   ├── Mutation pre-check (before execution)
│   └── Query post-filter (row-level security)
├── Admin API:
│   ├── Create/manage API tokens
│   ├── User role assignment
│   └── Grant CRUD
└── Auth integration tests
```

### Phase 5: Assets & Media (Week 14-15)

**Deliverables**: Image/file upload, storage, serving, metadata extraction.

```
Tasks:
├── Storage abstraction trait:
│   ├── LocalFileStorage (disk)
│   └── S3Storage (AWS S3 / MinIO)
├── Upload pipeline:
│   ├── Multipart form parsing (axum::extract::Multipart)
│   ├── SHA256 content hash (deduplication)
│   ├── Generate Sanity-compatible asset ID
│   ├── Store file → storage backend
│   └── Create asset document in database
├── Image processing:
│   ├── Metadata extraction (dimensions, format)
│   ├── On-the-fly resize/crop (via image crate)
│   └── Format conversion (WebP output)
├── Serving:
│   ├── CDN-compatible URL scheme
│   ├── Cache headers (ETag, Cache-Control)
│   └── Image transform query params (?w=800&h=600&fit=crop)
└── Asset management endpoints
```

### Phase 6: Presence System (Week 16)

**Deliverables**: Real-time presence via WebSocket.

```
Tasks:
├── WebSocket upgrade handler
├── Presence message protocol:
│   ├── state: { locations: [...] }
│   ├── rollCall: request all sessions to report
│   └── disconnect: session leaving
├── In-memory presence store:
│   ├── HashMap<SessionId, PresenceState>
│   ├── Auto-expire after 60s inactivity
│   └── Broadcast state changes
├── Integration with auth (extract user from WS handshake)
└── Presence API tests
```

### Phase 7: History & Versions (Week 17-18)

**Deliverables**: Transaction history API, document versioning, content releases.

```
Tasks:
├── Transaction log query API:
│   ├── GET /v1/data/history/{dataset}/transactions
│   ├── Filter by document ID, time range, author
│   └── Pagination (cursor-based)
├── Document-at-revision reconstruction:
│   └── Replay mutations from transaction log
├── Content releases:
│   ├── Release CRUD (create, update, delete, publish, schedule)
│   ├── Version document management (versions.{releaseId}.{docId})
│   ├── Publish release → promote all versions to published
│   └── Schedule release → timer-based publish
└── History integration tests
```

### Phase 8: Hardening & Production Readiness (Week 19-20)

```
Tasks:
├── Rate limiting (tower-governor or custom)
├── Request size limits (body size, query complexity)
├── GROQ query complexity limits (AST depth, result set size)
├── Connection pool tuning & health checks
├── Graceful shutdown (drain connections, flush events)
├── Metrics (prometheus / opentelemetry)
├── Structured error responses matching Sanity's error format
├── Database vacuum / maintenance guidance
├── Load testing (k6 or drill)
├── Security audit checklist
├── Docker production image (multi-stage, distroless)
├── Helm chart / docker-compose.prod.yml
└── Documentation (API reference, deployment guide)
```

---

## 9. Key Design Decisions

### 9.1 JSONB vs Normalized Tables

**Decision**: JSONB for document content.

**Rationale**: Sanity documents are schema-flexible (any fields). JSONB gives us:
- Zero-migration schema evolution
- GIN index for containment queries (`@>`)
- Native JSON path extraction for GROQ → SQL
- Matches Sanity's "store anything" philosophy

**Trade-off**: Joins are more expensive. Mitigated by GROQ → SQL transpilation using JSONB path operators.

### 9.2 Mutation Execution: In-Memory vs Pure SQL

**Decision**: Hybrid — fetch document, apply patch in Rust, write back.

**Rationale**: `diffMatchPatch`, complex array `insert`, and `setIfMissing` on nested paths are impractical in pure SQL. The `SELECT FOR UPDATE` → mutate → `UPDATE` pattern within a transaction gives us correctness with acceptable performance.

### 9.3 Event Bus: Single-Node vs Distributed

**Decision**: Start with `tokio::broadcast`, add PostgreSQL `LISTEN/NOTIFY` for multi-node.

**Rationale**: `tokio::broadcast` is zero-latency for single-node. `LISTEN/NOTIFY` is built into Postgres and requires no additional infrastructure. Redis Streams are a future option if needed.

### 9.4 GROQ Transpilation vs In-Memory Evaluation

**Decision**: SQL transpilation for dataset queries, in-memory evaluation for grant filters.

**Rationale**: SQL transpilation pushes filtering to Postgres (indexed). Grant filters run against a single known document — in-memory is simpler and faster for that case.

### 9.5 Soft Delete

**Decision**: `deleted BOOLEAN` column with filtered queries.

**Rationale**: Enables undo, audit trail, and matches Sanity's behavior where deletes are recorded in the transaction log.

---

## 10. Performance Targets

| Metric | Target | Strategy |
|--------|--------|----------|
| Query latency (p99) | < 10ms | JSONB GIN index, connection pool, SQL transpilation |
| Mutation latency (p99) | < 20ms | Single-round-trip transactions, prepared statements |
| SSE event delivery | < 50ms from commit | In-process broadcast, no serialization overhead |
| Concurrent listeners | 10,000+ | Tokio async, SSE (no thread-per-connection) |
| Document throughput | 5,000 mutations/sec | Connection pool, batch mutations in single TX |
| Startup time | < 500ms | Static binary, lazy initialization |

---

## 11. Testing Strategy

| Layer | Tool | Coverage Target |
|-------|------|-----------------|
| Unit (mutation logic) | `#[cfg(test)]` | 95% of core crate |
| Unit (GROQ parser) | `#[cfg(test)]` | 100% of grammar |
| Integration (API routes) | `axum::test` + `sqlx::test` | All routes, happy + error paths |
| Contract (Studio compat) | Custom harness | Replay Sanity Studio requests against our API |
| Load | `k6` or `drill` | Sustained 5k req/s |
| Fuzz | `cargo-fuzz` | GROQ parser, mutation input |

---

## 12. Risk Register

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| GROQ language complexity | High | High | Start with core subset (80/20), expand iteratively. Use Sanity's GROQ test suite. |
| Mendoza effect format undocumented | Medium | High | Reverse-engineer from `@sanity/diff` and listener events. Fallback: send full document in effects. |
| Studio version drift | Medium | Medium | Pin Studio version. Integration test suite catches regressions. |
| PostgreSQL JSONB query performance at scale | Medium | Low | GIN indexes, query plan analysis, partial indexes on common `_type` values. |
| Presence at scale (10k+ concurrent) | Low | Low | In-memory store is fast. Shard by project if needed. |

---

## 13. Milestone Summary

| Phase | Weeks | Deliverable | Studio Compatibility |
|-------|-------|-------------|---------------------|
| 0 | 1-2 | Server skeleton, DB, CI | None |
| 1 | 3-5 | CRUD + Mutations | Can create/edit/delete documents |
| 2 | 6-9 | GROQ Engine | Can query + list documents |
| 3 | 10-11 | SSE Listeners | Real-time updates work |
| 4 | 12-13 | Auth + Permissions | Login + access control |
| 5 | 14-15 | Assets | Image/file uploads |
| 6 | 16 | Presence | Collaboration indicators |
| 7 | 17-18 | History + Versions | Full document lifecycle |
| 8 | 19-20 | Production hardening | Production-ready |

**Total estimate**: ~20 weeks for a single senior engineer. Parallelizable to ~12 weeks with 2 engineers (split GROQ engine from mutation pipeline).

---

## 14. Dependency Graph (Build Order)

```
Phase 0 (Foundation)
    │
    ▼
Phase 1 (Mutations) ──────────┐
    │                          │
    ▼                          ▼
Phase 2 (GROQ) ◄──── Phase 4 (Auth) uses GROQ for grants
    │                          │
    ▼                          │
Phase 3 (Listeners)            │
    │                          │
    ├──────────────────────────┘
    ▼
Phase 5 (Assets)      — independent, can parallel with 3/4
Phase 6 (Presence)    — independent, can parallel with 5
Phase 7 (History)     — depends on 1, 2, 3
Phase 8 (Hardening)   — after all above
```

---

## 15. Getting Started Commands

```bash
# Initialize workspace
cargo init --name content-lake-rs
mkdir -p crates/{api,core,groq} migrations tests benches

# Add dependencies (api/Cargo.toml)
cargo add axum tokio --features full
cargo add sqlx --features postgres,runtime-tokio,tls-rustls,json,uuid,time
cargo add serde serde_json --features derive
cargo add tower tower-http --features cors,trace
cargo add tracing tracing-subscriber
cargo add uuid --features v7
cargo add jsonwebtoken argon2
cargo add dotenvy config
cargo add thiserror anyhow

# Setup database
docker compose up -d postgres
sqlx database create
sqlx migrate run

# Run
cargo run --bin content-lake-api
```
