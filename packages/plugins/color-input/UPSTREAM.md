# Upstream

Vendored from: https://github.com/sanity-io/color-input.git
Commit: 350af8aa70f582ddb26c21da7ae5c0d413425f0b
Vendored on: 2026-04-14

To re-sync:

```bash
cd packages/plugins/color-input
git init && git remote add upstream https://github.com/sanity-io/color-input.git
git fetch upstream
git reset --hard upstream/main
rm -rf .git
```
