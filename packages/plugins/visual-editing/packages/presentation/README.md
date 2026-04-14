> [!WARNING]  
> This package is replaced by [`sanity/presentation`].

## Migrate to [`sanity/presentation`]

Replace the dependency:

```sh
npm uninstall @sanity/presentation
pnpm add sanity@latest
```

Replace import statements:

```diff
-import { presentationTool } from '@sanity/presentation'
+import { presentationTool } from 'sanity/presentation'
```
