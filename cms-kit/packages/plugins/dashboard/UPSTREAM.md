# Upstream

Vendored from: https://github.com/sanity-io/dashboard.git
Commit: e5003ffccfdd531c8c513ca51c7eac1572d545df
Vendored on: 2026-04-14

To re-sync:

```bash
cd packages/plugins/dashboard
git init && git remote add upstream https://github.com/sanity-io/dashboard.git
git fetch upstream
git reset --hard upstream/main
rm -rf .git
```
