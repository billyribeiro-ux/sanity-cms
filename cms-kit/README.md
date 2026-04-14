# cms-kit

A personal toolkit for spinning up Sanity Studio projects quickly across clients.
Uses Sanity's free hosted Content Lake as the backend — no fork, no rebrand, no
self-hosting.

## What's in the box

```
cms-kit/
├── packages/
│   ├── schemas/          # Reusable schema types (page, SEO, image, CTA, etc.)
│   ├── studio-config/    # defineKitConfig() helper — your Studio defaults in one call
│   └── plugins/
│       └── vision/       # Vendored copy of @sanity/vision, yours to modify
└── templates/
    └── starter/          # Complete Studio you can copy into a new client repo
```

## How you use this across client projects

### Option A — per-client repo, installs from your registry

```bash
# In each client repo
npm create sanity@latest
pnpm add @cms-kit/schemas @cms-kit/studio-config @cms-kit/vision
```

Then in `sanity.config.ts`:

```ts
import {defineKitConfig} from '@cms-kit/studio-config'
import * as schemas from '@cms-kit/schemas'

export default defineKitConfig({
  projectId: 'xxxxxxxx',
  dataset: 'production',
  schemaTypes: Object.values(schemas),
})
```

### Option B — copy the starter

```bash
cp -r cms-kit/templates/starter my-client-studio
cd my-client-studio
# edit sanity.config.ts with projectId + dataset
pnpm install && pnpm dev
```

## Status

- ✅ Scaffolded
- ⏳ Schema types are placeholders — fill with real types as you build them
- ⏳ Studio config is a thin wrapper — extend with your defaults
- ⏳ Not yet published to a registry. Publish under your scope when ready.

## Renaming to your own npm scope

All packages are namespaced `@cms-kit/*` as a placeholder. When you pick a
real product name, do a global replace:

```bash
# from cms-kit/ root
grep -rln "@cms-kit/" . --exclude-dir=node_modules | xargs sed -i '' 's|@cms-kit/|@yourname/|g'
```

## Relationship to Sanity.io

This project uses Sanity Studio (MIT) and talks to Sanity's hosted Content Lake.
See [NOTICE](./NOTICE) for attribution. "Sanity" is a Sanity.io trademark; this
project is not affiliated with or endorsed by Sanity.io.
