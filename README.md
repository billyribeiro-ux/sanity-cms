# cms-kit

A personal, fully self-hosted headless CMS built on Sanity Studio + a Rust
Content Lake replica. **Nothing talks to `api.sanity.io`.** All data stays
on your Postgres, behind your Rust server, rendered by a Studio you control.

## What's in the box

```
sanity-cms/
├── packages/
│   ├── schemas/              Reusable Sanity schema types
│   ├── studio-config/        defineKitConfig() helper
│   └── plugins/              17 vendored first-party Sanity plugins
├── templates/
│   └── starter/              Ready-to-run Studio wired to the backend
└── content-lake-rs/          Rust backend (Axum + SQLx + Postgres)
    ├── crates/
    │   ├── api/              HTTP server
    │   ├── core/             Document repo, mutation engine, auth
    │   └── groq/             GROQ parser + SQL generator
    ├── migrations/
    ├── smoke-test.sh         End-to-end endpoint verification
    ├── Dockerfile
    └── docker-compose.yml
```

## Quick start

Requires: Node 20+, pnpm 10+, Rust 1.75+, Docker.

```bash
# Terminal 1 — backend
cd content-lake-rs
docker compose up -d postgres
cargo run --bin content-lake-api
# API is now at http://localhost:3030

# Terminal 2 — Studio
pnpm install
cp templates/starter/.env.example templates/starter/.env
pnpm --filter starter dev
# Studio opens at http://localhost:3333 and talks ONLY to localhost:3030
```

Log in with the default admin credentials (printed on first startup):

```
email:    admin@localhost
password: admin
```

⚠️ Set `ADMIN_EMAIL` / `ADMIN_PASSWORD` env vars before the first run for
anything beyond local dev.

## Verify everything works

After both terminals are running, in a third terminal:

```bash
cd content-lake-rs
./smoke-test.sh
```

The script exercises every endpoint (auth, projects, mutations, doc read,
GROQ query, SSE listen, asset upload, delete) and fails loudly on any
regression. Fully passes = backend is correctly replicating the Sanity
Content Lake contract for the implemented surface.

## What's implemented

### Backend (`content-lake-rs`) — 93 tests, zero failures

| Endpoint | Status |
|---|---|
| `GET /health`, `GET /v1/ping` | ✅ |
| `POST /v1/auth/login/password` | ✅ real JWT (argon2id + 7-day expiry) |
| `GET /v1/users/me`, `GET /v1/users/{id}` | ✅ JWT-gated |
| `GET /v1/auth/providers`, `POST /v1/auth/logout` | ✅ |
| `GET /v1/projects/{id}`, `GET /v1/projects/{id}/datasets` | ✅ |
| `GET /v1/data/doc/{dataset}/{ids}` | ✅ multi-id with `omitted` |
| `POST /v1/data/mutate/{dataset}` | ✅ create, createOrReplace, createIfNotExists, delete, patch (set, setIfMissing, unset, inc, dec, insert, merge, ifRevisionID, diffMatchPatch) |
| `GET /v1/data/query/{dataset}` | ✅ GROQ MVP subset (filter, order, slice, projection, params, `defined()`) |
| `POST /v1/data/query/{dataset}` | ✅ |
| `GET /v1/data/listen/{dataset}` | ✅ SSE with welcome + mutation events |
| `POST /v1/assets/images/{dataset}` | ✅ raw body, SHA-1 content addressing |
| `GET /assets/images/{filename}` | ✅ |

### Studio (`packages/`, `templates/starter/`)
- ✅ 7 reusable schema types (page, siteSettings, navigation, seo, imageWithAlt, link, cta, richText)
- ✅ `defineKitConfig()` with singleton handling + `apiHost` override
- ✅ 17 vendored first-party Sanity plugins under `packages/plugins/`
- ✅ Runnable starter pointing at `http://localhost:3030`

### Still honestly pending

- **GROQ extensions**: dereferencing (`->`), `in`, `match`, `count()`, subqueries — all return `400 unsupported`
- **Multipart asset upload** — raw body only; some Sanity client paths use multipart
- **Presence WebSocket** — multi-user cursors; skipped for MVP
- **Server-side schema validation** — Studio validates client-side only
- **Hardcoded-URL escape audit** of `@sanity/client` — most CDN/auth redirect URLs route via `apiHost`, but some may still reach `sanity.io`; catch via smoke-test

## Per-client workflow

```bash
cp -r templates/starter ~/projects/client-xyz
cd ~/projects/client-xyz
# Edit .env with SANITY_STUDIO_API_HOST, projectId, dataset
pnpm install
pnpm dev
```

Each client gets their own Postgres (or their own dataset on a shared
instance). Zero dependency on Sanity.io.

## Environment variables

### Backend
| Var | Default | Purpose |
|---|---|---|
| `DATABASE_URL` | (required) | Postgres DSN |
| `HOST`, `PORT` | `0.0.0.0`, `3030` | Bind address |
| `BOOTSTRAP_PROJECT`, `BOOTSTRAP_DATASET` | `default`, `production` | Auto-created on startup |
| `ADMIN_EMAIL`, `ADMIN_PASSWORD` | `admin@localhost`, `admin` | Bootstrap admin (WARN if default) |
| `JWT_SECRET` | `dev-secret-change-me-in-production` | **rotate this before deploying** |
| `AUTH_DISABLED` | `false` | Skip JWT verification (local only) |
| `ASSETS_DIR` | `./data/assets` | Uploaded file storage |
| `PUBLIC_BASE_URL` | `http://localhost:3030` | Absolute asset URLs |
| `LOG_LEVEL` | `info` | Tracing filter |

### Studio
| Var | Default | Purpose |
|---|---|---|
| `SANITY_STUDIO_PROJECT_ID` | `default` | Must match backend's `BOOTSTRAP_PROJECT` |
| `SANITY_STUDIO_DATASET` | `production` | Must match backend's `BOOTSTRAP_DATASET` |
| `SANITY_STUDIO_API_HOST` | `http://localhost:3030` | Unset to use Sanity's hosted backend |

## Renaming `@cms-kit/*` to your own scope

```bash
grep -rln "@cms-kit/" . --exclude-dir=node_modules --exclude-dir=target \
  | xargs sed -i 's|@cms-kit/|@yourname/|g'
```

## Licenses + attribution

- Studio code derives from [sanity-io/sanity](https://github.com/sanity-io/sanity) (MIT).
- Vendored plugins each carry their original LICENSE and an `UPSTREAM.md`
  noting the source commit SHA so you can re-sync later.
- `content-lake-rs` is original work.

See [NOTICE](./NOTICE) for the full attribution notice. "Sanity" is a
Sanity.io trademark; this project is not affiliated with or endorsed by
Sanity.io.
