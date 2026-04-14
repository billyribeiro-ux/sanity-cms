# Upstream

Vendored from: https://github.com/sanity-io/document-internationalization.git
Commit: dc1a9cbd676c82b254fc1eeb43fc0024d7440de6
Vendored on: 2026-04-14

To re-sync:

```bash
cd packages/plugins/document-internationalization
git init && git remote add upstream https://github.com/sanity-io/document-internationalization.git
git fetch upstream
git reset --hard upstream/main
rm -rf .git
```
