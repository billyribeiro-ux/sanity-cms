# Vendored Sanity Plugins

All first-party `sanity-io/*` plugins, cloned and checked in so you can modify them.
Each folder has an `UPSTREAM.md` with the origin URL and commit SHA it was pulled from.

| Folder | Upstream | What it does |
|---|---|---|
| `vision/` | sanity-io/sanity (@sanity/vision) | GROQ query playground inside the Studio |
| `code-input/` | sanity-io/code-input | Code editor field with syntax highlighting |
| `color-input/` | sanity-io/color-input | Color picker field |
| `table/` | sanity-io/table | Table field |
| `google-maps-input/` | sanity-io/google-maps-input | Google Maps location picker |
| `dashboard/` | sanity-io/dashboard | Dashboard tool with widgets |
| `document-internationalization/` | sanity-io/document-internationalization | Document-level translations |
| `language-filter/` | sanity-io/language-filter | Per-field language filtering |
| `orderable-document-list/` | sanity-io/orderable-document-list | Drag-to-reorder document lists |
| `sanity-plugin-mux-input/` | sanity-io/sanity-plugin-mux-input | Mux video uploads |
| `sanity-plugin-asset-source-unsplash/` | sanity-io/sanity-plugin-asset-source-unsplash | Unsplash image picker |
| `sanity-plugin-iframe-pane/` | sanity-io/sanity-plugin-iframe-pane | Live preview iframe pane |
| `sanity-plugin-media/` | sanity-io/sanity-plugin-media | Full media library browser |
| `sanity-plugin-internationalized-array/` | sanity-io/sanity-plugin-internationalized-array | Per-field language arrays |
| `sanity-plugin-markdown/` | sanity-io/sanity-plugin-markdown | Markdown field |
| `sanity-plugin-hotspot-array/` | sanity-io/sanity-plugin-hotspot-array | Image hotspot arrays (tagged points on images) |
| `visual-editing/` | sanity-io/visual-editing | Presentation tool, live preview, preview-url-secret (monorepo) |

## Re-syncing a plugin

```bash
cd packages/plugins/<plugin>
git init && git remote add upstream $(cat UPSTREAM.md | grep "Vendored from:" | cut -d' ' -f3-)
git fetch upstream
git reset --hard upstream/main   # or master, check the repo
rm -rf .git
```

Then review the diff against your local changes.

## Using a vendored plugin in the starter

Each plugin is a full package with its own `package.json`. You can either:

1. **Install as workspace dependency** — add `"@cms-kit/vision": "workspace:*"` after renaming
   the package names in their `package.json` files.
2. **Install from source** — `pnpm add file:../../packages/plugins/vision`.
3. **Pick what you need** — copy the `src/` of a plugin into your Studio project directly.

## What's NOT here

- **`@sanity/cli`, `@sanity/types`, `@sanity/schema`, `@sanity/mutator`, `sanity`** — these
  are the Studio core. Don't vendor. Use upstream via npm.
- **`@sanity/client`** — the JS SDK for talking to Content Lake. Already an npm package.
- **Hosted services** — Content Lake, Studio hosting, Image CDN, Scheduled Publishing API.
  These are Sanity.io SaaS and have no open-source equivalent here.
