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
    │   ├── core/             Document repo, mutation engine
    │   └── groq/             GROQ parser (query layer, in progress)
    ├── migrations/
    ├── Dockerfile
    └── docker-compose.yml
```

## Quick start

Requires: Node 20+, pnpm 10+, Rust 1.75+, Docker.

```bash
# 1. Start Postgres + the Rust API
cd content-lake-rs
docker compose up -d postgres
cargo run --bin content-lake-api
# API is now at http://localhost:3030

# 2. In another terminal: boot the Studio
cd ..
pnpm install
cp templates/starter/.env.example templates/starter/.env
pnpm --filter starter dev
# Studio opens at http://localhost:3333 and talks ONLY to localhost:3030
```

## What's implemented

### Backend (`content-lake-rs`)
- ✅ `GET /health`, `GET /v1/ping`
- ✅ `GET /v1/users/me`, `GET /v1/users/{id}`, `GET /v1/auth/providers`, `POST /v1/auth/logout` (single-user stubs)
- ✅ `GET /v1/projects/{id}`, `GET /v1/projects/{id}/datasets`
- ✅ `GET /v1/data/doc/{dataset}/{ids}` — multi-id document fetch with `omitted` reporting
- ✅ `POST /v1/data/mutate/{dataset}` — full mutation batch: create, createOrReplace,
  createIfNotExists, delete, patch (set, setIfMissing, unset, inc, dec, insert,
  merge, ifRevisionID)
- ✅ Postgres schema with JSONB documents, transaction log
- ✅ Bootstrap default project/dataset on startup
- ⏳ `GET /v1/data/query/{dataset}` (GROQ → SQL over JSONB) — parser built, query planner pending
- ⏳ `GET /v1/data/listen/{dataset}` (SSE real-time) — event bus in place
- ⏳ `POST /v1/assets/images/{dataset}` — asset uploads
- ⏳ `diffMatchPatch` patch op — currently returns `Unsupported`
- ⏳ Real auth (token-based) — currently single-user trust

### Studio (`packages/`, `templates/starter/`)
- ✅ 7 reusable schema types (page, siteSettings, navigation, seo, imageWithAlt, link, cta, richText)
- ✅ `defineKitConfig()` with singleton handling + apiHost override
- ✅ 17 vendored first-party Sanity plugins under `packages/plugins/`
- ✅ Runnable starter pointing at `http://localhost:3030`

## Per-client project workflow

```bash
# Bootstrap a new client project
cp -r templates/starter ~/projects/client-xyz
cd ~/projects/client-xyz
# Set the projectId + dataset in .env (or create them via the backend first)
pnpm install
pnpm dev
```

Every client gets their own Postgres database (or their own dataset on a
shared database). Zero dependency on Sanity.io infrastructure.

## Renaming `@cms-kit/*` to your own scope

Packages are namespaced `@cms-kit/*` as a placeholder. Global replace when you
pick a product name:

```bash
grep -rln "@cms-kit/" . --exclude-dir=node_modules --exclude-dir=target \
  | xargs sed -i 's|@cms-kit/|@yourname/|g'
```

## Licenses + attribution

- Studio code derives from [sanity-io/sanity](https://github.com/sanity-io/sanity) (MIT).
- Vendored plugins each carry their original LICENSE and an `UPSTREAM.md` noting
  the source commit SHA so you can re-sync later.
- `content-lake-rs` is original work.

See [NOTICE](./NOTICE) for the full attribution notice. "Sanity" is a Sanity.io
trademark; this project is not affiliated with or endorsed by Sanity.io.
