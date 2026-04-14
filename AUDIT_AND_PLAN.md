# End-to-End Audit & Implementation Plan

**Repo:** `billyribeiro-ux/sanity-cms`
**Branch:** `claude/audit-codebase-report-58Pt8`
**Date:** 2026-04-14

---

## Part 1 — What You Actually Have

Your repo is a **two-project monorepo** living under one GitHub repo:

| Path | What it is | Origin | State |
|---|---|---|---|
| `sanity-main/` | Full Sanity Studio monorepo (packages/sanity, @sanity/cli, @sanity/types, @sanity/schema, @sanity/mutator, etc.) | Copy of upstream `sanity-io/sanity` @ v5.12.0, MIT | ~43 MB, unmodified from upstream except top-level additions (`RUST_API_PLAN.md`, `.claude/`, `skills/`, `AGENTS.md`) |
| `content-lake-rs/` | Your custom Rust backend (Axum + SQLx + Postgres) targeting the Sanity Content Lake HTTP contract | Written by you (11 commits, Billy Ribeiro) | **~1,862 LOC, Phase 0.** Health endpoints only. ~965 LOC of GROQ lexer/parser/evaluator. No real document/mutate/query routes yet. |

### Key facts that drive the plan

1. **License is MIT** (`sanity-main/LICENSE`). You are legally allowed to fork, modify, rebrand, sell, and sublicense — *provided* you keep the MIT notice for Sanity.io's original code.
2. **"Sanity" is a trademark** owned by Sanity.io. MIT ≠ trademark license. You cannot ship a product called "Sanity" or call yourself a Sanity authorized distribution.
3. **You are not actually forked from upstream in git.** Your `origin` is just `billyribeiro-ux/sanity-cms`. There is no `upstream` remote, no fork relationship, and the upstream git history is not present (11 commits total, starting with "Initial commit" dumping the source tree in). This means *upstream sync is currently impossible* without work.
4. **The Studio has ~1,843 files importing `@sanity/*`** and ~30 hardcoded references to `api.sanity.io` / `api.sanity.work`. A full rebrand is a very large, mechanical change.
5. **content-lake-rs is early.** The implementation plan in `RUST_API_PLAN.md` spans 6 phases; you're at Phase 0 with only GROQ lexing/parsing/in-memory eval started. No storage, no mutations, no listeners, no auth, no assets.

### Risks in current state

- ❌ **No upstream sync path** — you'll drift from Sanity.io fixes/features with no way to merge them.
- ❌ **Branding collision** — if you ship this to a client as-is, every package name, error message, doc URL, and asset points to Sanity.io.
- ❌ **Legal exposure** — a client product that reads "Sanity Studio" in the UI and ships `@sanity/cli` could trigger trademark claims even though the code is MIT.
- ❌ **content-lake-rs is far from usable** — it cannot back a real Studio yet. Today, your "fork" still depends on Sanity's hosted Content Lake to do anything.
- ⚠️ **Two unrelated projects in one repo** — forks you into a hard monorepo problem (two languages, two release cadences, two CI pipelines).
- ⚠️ **No CI, no tests running in this repo root** — everything is inherited from the Sanity Studio subdir.

### Strengths

- ✅ MIT license genuinely gives you the freedom you want.
- ✅ Studio code is modular (packages are well-separated).
- ✅ Good foundation documents already exist (`ARCHITECTURE.md`, `AGENTS.md`, `RUST_API_PLAN.md`).
- ✅ Rust GROQ parser is a solid start — arguably the hardest part of the backend.

---

## Part 2 — Strategy: Pick Your Level

You have three realistic levels of "making it yours." Pick one explicitly before writing code — they compound but shouldn't be blended.

### Level A — Private Internal Toolkit (fastest, lowest risk)
Use the Studio as-is under MIT, build **plugins + Studio config templates** you reuse across clients. Do *not* rebrand. Do *not* fork packages. Treat Sanity.io as your backend.

- Time: days
- Maintenance: near-zero (you consume upstream npm releases)
- Trade-off: You're not independent from Sanity.io infra, and every client still pays Sanity.io.

### Level B — Rebranded Self-Hosted Studio (medium, recommended starting point)
Rebrand the Studio to your own name, publish packages to a **private npm scope** (e.g. `@billyribeiro/*` or `@yourclient-cms/*`), and run it against your own `content-lake-rs` backend. Keep upstream sync alive via a proper fork remote.

- Time: 4–8 weeks realistic for an MVP rebrand + Phase 1 of content-lake-rs
- Maintenance: moderate (monthly upstream merges)
- Trade-off: You own the brand and the data plane, but you must maintain the backend.

### Level C — Permanent Hard Fork (hardest, only if you have a product reason)
Diverge permanently — remove Sanity-specific concepts, add your own, stop syncing upstream. This is a real CMS product, not a toolkit.

- Time: quarters / years
- Maintenance: a full-time job
- Trade-off: Complete ownership. No upstream safety net.

**Recommendation: Level B.** It gets you the independence you're asking for without committing to rewriting a CMS.

---

## Part 3 — The Implementation Plan (Level B)

### Phase 0 — Legal & hygiene (1 day, do first)

1. **Keep upstream MIT notice intact.** In any rebranded package, append your own copyright below Sanity.io's, never replace it:
   ```
   Copyright (c) 2016–2026 Sanity.io
   Copyright (c) 2026 Billy Ribeiro
   ```
2. **Add a NOTICE file** at repo root explaining the provenance: "This project is derived from Sanity Studio (MIT, © Sanity.io). Trademarks are the property of their respective owners."
3. **Pick a product name.** Must not contain "Sanity." Suggested working names: `Lakehouse`, `Hydra CMS`, `Ribeiro Studio`, or a per-client name. Reserve the npm scope immediately (e.g. `@billyribeiro`, `@lakehouse-cms`).
4. **Split the repo** into two repos OR keep a monorepo but give each project its own release pipeline. Do not ship `sanity-main/` and `content-lake-rs/` from a single version.
5. **Establish upstream remote** on the Studio fork:
   ```
   git remote add upstream https://github.com/sanity-io/sanity.git
   git fetch upstream
   ```
   Rebase your `main` onto upstream's tag v5.12.0 so future merges are clean. This is one-time pain that unblocks Level B entirely.

### Phase 1 — Studio rebrand skeleton (1–2 weeks)

Goal: A Studio binary you can `npx @yourscope/create-studio` that boots to your branding and talks to `localhost:3030` instead of `api.sanity.io`.

1. **Codemod the scopes.** Write a one-shot script (`scripts/rebrand.ts`) that:
   - Renames `@sanity/*` → `@yourscope/*` in all `package.json` files
   - Renames the `sanity` root package → `@yourscope/studio`
   - Rewrites workspace imports accordingly
   - Updates `pnpm-workspace.yaml` and `lerna.json`
   Commit the codemod so re-running it against future upstream merges is trivial.
2. **Rebrand user-visible strings only** (not internals):
   - Login screens, empty states, "Powered by Sanity" badges
   - CLI help text (`@sanity/cli` → `@yourscope/cli`)
   - The `sanity` binary name → your binary name
   Keep internal class names and GROQ terminology unchanged — they're technical vocabulary, not branding.
3. **Centralize API endpoints.** The ~30 hardcoded `api.sanity.io` references must route through a single config module (`packages/@yourscope/util/src/endpoints.ts`) that reads `STUDIO_API_HOST` env var. Default it to your future content-lake-rs URL, fall back to Sanity.io for dev.
4. **Strip what you won't maintain.** Delete `examples/`, `perf/`, `dev/efps/`, `dev/design-studio/`, and upstream-specific CI workflows. Keep `dev/test-studio/` as your smoke test.
5. **Publish to private registry.** Set up a GitHub Packages or Verdaccio registry under your scope. Wire `lerna publish` to it. Tag your first release `0.1.0-alpha.1`.

### Phase 2 — content-lake-rs to MVP-parity (6–10 weeks)

Goal: the Studio boots against content-lake-rs and you can create, edit, query, and list a document through the real Sanity Studio UI.

Follow the existing `RUST_API_PLAN.md` phases but reorder for *Studio unblocking*:

| Order | Endpoint | Why first |
|---|---|---|
| 1 | `POST /v1/auth/...` (mock initially) | Studio won't load without an auth handshake |
| 2 | `GET /v1/data/doc/{dataset}/{id}` | Opening any document |
| 3 | `POST /v1/data/mutate/{dataset}` (create, patch, delete, ifRevisionID) | Editing |
| 4 | `GET /v1/data/query/{dataset}` (GROQ → SQL over JSONB) | Document lists, references |
| 5 | `GET /v1/data/listen/{dataset}` (SSE) | Real-time updates; Studio degrades OK without this, but UX suffers |
| 6 | `POST /v1/assets/images/{dataset}` | Media — can stub to local disk or S3 |
| 7 | Presence (WS) | Nice-to-have; skip until multi-user |

Deliverables per endpoint: integration test that boots an actual Studio in Playwright and exercises the path. This is the only way you'll catch contract mismatches.

**Critical path: GROQ → SQL**. Your current evaluator is in-memory. For production you need a SQL generator over `jsonb`. Build this as two backends (in-memory for tests, Postgres for prod) behind a `QueryEngine` trait. Sanity's GROQ reference tests (public, MIT) should be your conformance suite.

### Phase 3 — Client/project reuse kit (2 weeks)

Goal: onboarding a new project takes <30 minutes.

1. **`create-studio` template.** A tiny CLI (one file, `tsx`) that scaffolds: Studio config, schema folder, Docker Compose with content-lake-rs + Postgres, `.env.example`, and a deploy script.
2. **Schema library.** Extract reusable schema types (page, SEO, image-with-alt, localized-string, etc.) into `@yourscope/schemas` — the single most reused piece across client projects.
3. **Deployment presets.** Terraform / Docker Compose / Fly.io configs for content-lake-rs. One command: `yourscope deploy`.
4. **Upgrade script.** `yourscope upgrade` that pulls the latest rebrand codemod output. Clients update with one command.

### Phase 4 — Upstream sync cadence (ongoing)

1. **Monthly**: `git fetch upstream && git merge upstream/main`. Your codemod handles the scope renames on new files automatically. Diffs should be minimal if Phase 1 was clean.
2. **Per upstream major**: re-run test-studio + Playwright contract tests against content-lake-rs. Bump your major.
3. **Do not patch upstream code in-place.** Every customization should be a plugin or config override living in a separate package. This is the single rule that keeps Level B sustainable.

### Phase 5 — Productization (if you want Level C later)

Only after Phases 1–4 are stable:
- Auth/billing/admin UI
- Multi-tenant hosting
- Marketing site and docs under your own domain
- At this point evaluate whether to diverge permanently.

---

## Part 4 — Concrete Next Actions (this week)

1. ☐ Pick Level A, B, or C. Write it in `README.md`.
2. ☐ Pick a product name and npm scope. Reserve the scope.
3. ☐ Split `sanity-main/` into its own repo with `upstream` remote configured; rebase onto upstream `v5.12.0`.
4. ☐ Move `content-lake-rs/` to its own repo too.
5. ☐ Add NOTICE + dual-copyright headers to your new repos.
6. ☐ Write the rebrand codemod (`scripts/rebrand.ts`) and dry-run it on a scratch branch.
7. ☐ Write one Playwright test that boots Studio against a stubbed content-lake-rs `/health` + `/v1/auth` and verifies it loads.

Once those seven items are done, you have a real platform to iterate on. Everything else in Phases 1–3 then becomes mechanical.

---

## Part 5 — Honest Cost Estimate

| Scope | Rough effort (solo) |
|---|---|
| Level A (toolkit only, no backend) | 1–2 weeks |
| Level B Phase 1 (rebrand + private publish) | 2–3 weeks |
| Level B Phase 2 (content-lake-rs MVP) | 2–4 months |
| Level B Phase 3 (client kit) | 2 weeks |
| Level B Phase 4 (ongoing sync) | ~4 hours/month |
| Level C (permanent fork) | Don't. Unless a client is paying for it. |

The honest read: **Level B Phase 1 gives you 80% of what you want in 3 weeks.** Phase 2 is the big commitment and should only start if a specific client need justifies self-hosting.
