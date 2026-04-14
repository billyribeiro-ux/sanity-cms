# Upstream

Vendored from: https://github.com/sanity-io/code-input.git
Commit: 0e757d597ab2100323846f5e53d81b42731b5d6e
Vendored on: 2026-04-14

To re-sync:

```bash
cd packages/plugins/code-input
git init && git remote add upstream https://github.com/sanity-io/code-input.git
git fetch upstream
git reset --hard upstream/main
rm -rf .git
```
