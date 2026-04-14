# Upstream

Vendored from: https://github.com/sanity-io/table.git
Commit: 02a08c6eab112d2d5f31f97ef80d132d1a14182d
Vendored on: 2026-04-14

To re-sync:

```bash
cd packages/plugins/table
git init && git remote add upstream https://github.com/sanity-io/table.git
git fetch upstream
git reset --hard upstream/main
rm -rf .git
```
