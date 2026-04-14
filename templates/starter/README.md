# Starter Studio

A minimal Sanity Studio wired up to `@cms-kit/schemas` and `@cms-kit/studio-config`.

## Use it

From the cms-kit workspace:

```bash
cp templates/starter/.env.example templates/starter/.env
# edit .env with your Sanity projectId + dataset
pnpm install
pnpm --filter starter dev
```

Studio opens at http://localhost:3333.

## Use it in a new client project

```bash
cp -r cms-kit/templates/starter ~/projects/client-xyz-studio
cd ~/projects/client-xyz-studio
# edit package.json name, set your projectId/dataset
pnpm install
pnpm dev
```

## Add client-specific schemas

Create a `schemas/` folder and pass them into `defineKitConfig`:

```ts
// sanity.config.ts
import {defineKitConfig} from '@cms-kit/studio-config'
import {product} from './schemas/product'
import {category} from './schemas/category'

export default defineKitConfig({
  projectId: '...',
  dataset: 'production',
  schemaTypes: [product, category],
  singletons: ['siteSettings'],
})
```
