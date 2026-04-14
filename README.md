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
| `GET /v1/data/query/{dataset}` | ✅ GROQ (filter, order, slice, projection, params, `defined()`, `in`, `match`, `count()`, `references()`, deref `->`) |
| `POST /v1/data/query/{dataset}` | ✅ |
| `GET /v1/data/listen/{dataset}` | ✅ SSE with welcome + mutation events |
| `POST /v1/assets/images/{dataset}` | ✅ raw body, SHA-1 content addressing |
| `GET /assets/images/{filename}` | ✅ |
| `POST /v1/assets/files/{dataset}` | ✅ sanity.fileAsset, multipart or raw |
| `GET /assets/files/{filename}` | ✅ |
| `GET /v1/presence/{dataset}` | ✅ WebSocket — welcome, initial snapshot, state + disappear broadcasts |

### Studio (`packages/`, `templates/starter/`)
- ✅ 7 reusable schema types (page, siteSettings, navigation, seo, imageWithAlt, link, cta, richText)
- ✅ `defineKitConfig()` with singleton handling + `apiHost` override
- ✅ 17 vendored first-party Sanity plugins under `packages/plugins/`
- ✅ Runnable starter pointing at `http://localhost:3030`

### Still honestly pending

- **GROQ gaps**: arbitrary subqueries in projections, slice-expressions in filters, `order()` on nested arrays, `pt::text()` and other string helpers
_None of the known gaps leak to Sanity.io — everything runs locally or fails with a 4xx._

## Server-side schema validation

Opt-in. Set `SCHEMA_FILE` to a JSON document describing your schemas and the
backend will reject mutations that don't match. Empty registry (the default)
runs zero validation — bring your own schemas when ready.

See `content-lake-rs/examples/schema.example.json` for a full example. Minimal shape:

```json
{
  "types": {
    "page": {
      "fields": [
        {"name": "title", "type": "string", "required": true, "max": 120},
        {"name": "slug",  "type": "object", "required": true,
         "fields": [{"name": "current", "type": "string", "pattern": "^[a-z0-9-]+$"}]},
        {"name": "body",  "type": "array", "of": ["block", "image"]}
      ]
    }
  }
}
```

Supported: string / number / boolean / null / datetime / reference / image /
block / object / array-of-types; constraints `required`, `min`, `max`, `value`
(exact), `pattern` (regex). Documents whose `_type` isn't in the registry
pass through unchanged — validation is opt-in per type.
- **Hardcoded-URL escape audit** of `@sanity/client` — most CDN/auth redirect URLs route via `apiHost`, but some may still reach `sanity.io`; catch via smoke-test

## Domain guard — why no traffic leaks to Sanity

Even with `apiHost` set, `@sanity/client` and `sanity` still hardcode a
handful of hosts for telemetry, Sentry, the Media Library, CDN fallback,
and Canvas links:

| Host | What it does |
|---|---|
| `api.sanity.io/vX/intake/tracing` | Telemetry + Sentry error-reporting tunnel |
| `apicdn.sanity.io` / `sanity-cdn.com` | CDN query fallback |
| `media.sanity.io` | Media Library frontend |
| `sentry.sanity.io` | Error reporting |
| `*.sanity.studio` | Studio hosting redirects |

`defineKitConfig({apiHost: '...'})` installs a global `fetch`/`XMLHttpRequest`
guard that rejects any request to those hosts with a `451 Blocked` response
and logs a warning. Every outbound call is either local or third-party of
your choice — nothing reaches Sanity's servers.

Override with `blockSanityDomains: false` if you deliberately want Media
Library or Canvas to reach Sanity.io. Add specific hosts back with
`installSanityDomainGuard({allow: ['media.sanity.io']})`.

Plain documentation `<a href>` links (the Studio's "Learn more" buttons etc.)
are unaffected — they're inert until a user clicks.

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
