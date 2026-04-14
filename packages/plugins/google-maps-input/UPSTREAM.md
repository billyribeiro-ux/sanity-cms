# Upstream

Vendored from: https://github.com/sanity-io/google-maps-input.git
Commit: 7f67298ec203afa0b1c4d65c42681c1b01f033e2
Vendored on: 2026-04-14

To re-sync:

```bash
cd packages/plugins/google-maps-input
git init && git remote add upstream https://github.com/sanity-io/google-maps-input.git
git fetch upstream
git reset --hard upstream/main
rm -rf .git
```
