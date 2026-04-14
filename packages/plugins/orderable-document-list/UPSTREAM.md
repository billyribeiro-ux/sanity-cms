# Upstream

Vendored from: https://github.com/sanity-io/orderable-document-list.git
Commit: ea04e91bf4ce9141cfe774bc323b1736ba3c058c
Vendored on: 2026-04-14

To re-sync:

```bash
cd packages/plugins/orderable-document-list
git init && git remote add upstream https://github.com/sanity-io/orderable-document-list.git
git fetch upstream
git reset --hard upstream/main
rm -rf .git
```
