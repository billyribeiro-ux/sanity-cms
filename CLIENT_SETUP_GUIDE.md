# Client Setup Guide

This guide shows how to spin up a new Studio app from this repo and connect it to your self-hosted backend.

## Prerequisites

- Node.js 20+
- `pnpm` 10+
- (Optional for local backend) Docker + Rust toolchain

## Option A: One-command bootstrap (recommended)

From the repo root:

```bash
bash ./bootstrap-client.sh --name client-xyz --install
```

This will:

- copy `templates/starter` into `~/projects/client-xyz`
- create `.env` from `.env.example`
- set:
  - `SANITY_STUDIO_PROJECT_ID=default`
  - `SANITY_STUDIO_DATASET=production`
  - `SANITY_STUDIO_API_HOST=http://localhost:3030`
- rename `package.json` name to `client-xyz`
- run `pnpm install` (when `--install` is passed)

### Useful flags

```bash
bash ./bootstrap-client.sh \
  --name client-acme \
  --target ~/work \
  --project-id acme \
  --dataset production \
  --api-host http://localhost:3030 \
  --install
```

## Option B: Manual setup

```bash
cp -R templates/starter ~/projects/client-xyz
cd ~/projects/client-xyz
cp .env.example .env
pnpm install
pnpm dev
```

Studio runs at `http://localhost:3333`.

## Run with local self-hosted backend

In one terminal:

```bash
cd content-lake-rs
docker compose up -d postgres
cargo run --bin content-lake-api
```

In another terminal:

```bash
cd ~/projects/client-xyz
pnpm dev
```

## Use in an existing Studio app

If you already have a Studio app and want to reuse this package set:

```bash
pnpm add @cms-kit/schemas @cms-kit/studio-config
```

Then use `defineKitConfig` in `sanity.config.ts`:

```ts
import {defineKitConfig} from '@cms-kit/studio-config'
import {product} from './schemas/product'

export default defineKitConfig({
  projectId: 'default',
  dataset: 'production',
  apiHost: 'http://localhost:3030',
  schemaTypes: [product],
  singletons: ['siteSettings'],
})
```

## Troubleshooting

- **`pnpm: command not found`**: install pnpm first (`corepack enable && corepack prepare pnpm@latest --activate`).
- **Studio cannot connect to API**: verify `SANITY_STUDIO_API_HOST` and that backend is running on that URL.
- **Port in use (`3333`)**: run `pnpm dev -- --port 3334`.
