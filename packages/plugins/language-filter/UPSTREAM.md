# Upstream

Vendored from: https://github.com/sanity-io/language-filter.git
Commit: 046dfc5300a2d3d3b430d9bb566961631116e7dd
Vendored on: 2026-04-14

To re-sync:

```bash
cd packages/plugins/language-filter
git init && git remote add upstream https://github.com/sanity-io/language-filter.git
git fetch upstream
git reset --hard upstream/main
rm -rf .git
```
