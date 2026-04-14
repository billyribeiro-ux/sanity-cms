#!/usr/bin/env bash
# End-to-end smoke test for the content-lake-rs API.
#
# Assumes:
#   - Postgres running (docker compose up -d postgres from this dir works)
#   - `cargo run --bin content-lake-api` running in another terminal
#
# Exits non-zero on any failure. Prints a human-readable report.

set -euo pipefail

API="${API:-http://localhost:3030}"
PROJECT="${PROJECT:-default}"
DATASET="${DATASET:-production}"
ADMIN_EMAIL="${ADMIN_EMAIL:-admin@localhost}"
ADMIN_PASSWORD="${ADMIN_PASSWORD:-admin}"

ok()   { printf "\033[32m✓\033[0m %s\n" "$1"; }
fail() { printf "\033[31m✗\033[0m %s\n" "$1"; echo "  $2"; exit 1; }
hdr()  { printf "\n\033[1m== %s ==\033[0m\n" "$1"; }

# ---- 0. basic reachability ----
hdr "health"
code=$(curl -s -o /dev/null -w "%{http_code}" "$API/health")
[ "$code" = "200" ] || fail "/health" "got $code"
ok "/health → 200"

code=$(curl -s -o /dev/null -w "%{http_code}" "$API/v1/ping")
[ "$code" = "200" ] || fail "/v1/ping" "got $code"
ok "/v1/ping → 200"

# ---- 1. auth ----
hdr "auth"

providers=$(curl -s "$API/v1/auth/providers")
echo "$providers" | grep -q '"password"' || fail "providers" "missing 'password'"
ok "/v1/auth/providers lists password provider"

login=$(curl -s -X POST "$API/v1/auth/login/password" \
  -H 'Content-Type: application/json' \
  -d "{\"email\":\"$ADMIN_EMAIL\",\"password\":\"$ADMIN_PASSWORD\"}")

TOKEN=$(echo "$login" | python3 -c 'import json,sys;print(json.load(sys.stdin).get("token",""))')
[ -n "$TOKEN" ] || fail "login" "no token in response: $login"
ok "/v1/auth/login/password issued JWT (${#TOKEN} chars)"

me=$(curl -s "$API/v1/users/me" -H "Authorization: Bearer $TOKEN")
echo "$me" | grep -q "\"email\":\"$ADMIN_EMAIL\"" || fail "me" "$me"
ok "/v1/users/me resolved via bearer token"

# ---- 2. project / dataset ----
hdr "project"
p=$(curl -s "$API/v1/projects/$PROJECT")
echo "$p" | grep -q '"displayName"' || fail "project" "$p"
ok "/v1/projects/$PROJECT returned metadata"

ds=$(curl -s "$API/v1/projects/$PROJECT/datasets")
echo "$ds" | grep -q "\"name\":\"$DATASET\"" || fail "datasets" "$ds"
ok "/v1/projects/$PROJECT/datasets lists $DATASET"

# ---- 3. mutations ----
hdr "mutate"

DOC_ID="smoke-test-$(date +%s)"

mutate=$(curl -s -X POST "$API/v1/data/mutate/$DATASET" \
  -H "Authorization: Bearer $TOKEN" \
  -H 'Content-Type: application/json' \
  -d "{\"mutations\":[{\"create\":{\"_id\":\"$DOC_ID\",\"_type\":\"page\",\"title\":\"Hello\"}}]}")
echo "$mutate" | grep -q "\"id\":\"$DOC_ID\"" || fail "create" "$mutate"
ok "create $DOC_ID"

patch=$(curl -s -X POST "$API/v1/data/mutate/$DATASET" \
  -H "Authorization: Bearer $TOKEN" \
  -H 'Content-Type: application/json' \
  -d "{\"mutations\":[{\"patch\":{\"id\":\"$DOC_ID\",\"set\":{\"title\":\"Updated\"}}}]}")
echo "$patch" | grep -q '"operation":"update"' || fail "patch" "$patch"
ok "patch $DOC_ID.title → 'Updated'"

# ---- 4. reads ----
hdr "read"
doc=$(curl -s "$API/v1/data/doc/$DATASET/$DOC_ID")
echo "$doc" | grep -q '"title":"Updated"' || fail "doc fetch" "$doc"
ok "/v1/data/doc/$DATASET/$DOC_ID returns updated title"

# ---- 5. GROQ query ----
hdr "query"
q=$(python3 -c "import urllib.parse;print(urllib.parse.quote('*[_type == \"page\"]'))")
resp=$(curl -s "$API/v1/data/query/$DATASET?query=$q" \
  -H "Authorization: Bearer $TOKEN")
echo "$resp" | grep -q "\"$DOC_ID\"" || fail "query" "$resp"
echo "$resp" | grep -q '"ms"' || fail "query ms" "$resp"
ok "GROQ *[_type == \"page\"] returned the document"

# POST form
qp=$(curl -s -X POST "$API/v1/data/query/$DATASET" \
  -H "Authorization: Bearer $TOKEN" \
  -H 'Content-Type: application/json' \
  -d "{\"query\":\"*[_type == \\\"page\\\"]{_id, title}\",\"params\":{}}")
echo "$qp" | grep -q "\"title\":\"Updated\"" || fail "query POST" "$qp"
ok "POST query with projection returned {_id, title}"

# ---- 6. SSE listen ----
hdr "listen (SSE)"
# Spawn a curl backgrounded, capture one welcome event then kill.
SSE_LOG=$(mktemp)
timeout 4 curl -s -N "$API/v1/data/listen/$DATASET" \
  -H "Authorization: Bearer $TOKEN" > "$SSE_LOG" &
SSE_PID=$!

sleep 2
# Trigger a mutation so the stream emits something
curl -s -X POST "$API/v1/data/mutate/$DATASET" \
  -H "Authorization: Bearer $TOKEN" \
  -H 'Content-Type: application/json' \
  -d "{\"mutations\":[{\"patch\":{\"id\":\"$DOC_ID\",\"set\":{\"title\":\"SSE test\"}}}]}" > /dev/null

wait $SSE_PID 2>/dev/null || true
grep -q "event: welcome" "$SSE_LOG" || fail "SSE welcome" "$(cat $SSE_LOG)"
ok "SSE emitted 'welcome'"
grep -q "event: mutation" "$SSE_LOG" || fail "SSE mutation" "$(cat $SSE_LOG)"
ok "SSE emitted 'mutation' after a mutate"
rm -f "$SSE_LOG"

# ---- 7. asset upload ----
hdr "assets"
IMG=$(mktemp --suffix=.png)
# 1x1 transparent PNG (68 bytes)
printf '\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x06\x00\x00\x00\x1f\x15\xc4\x89\x00\x00\x00\rIDATx\x9cc\xf8\x0f\x00\x01\x01\x01\x00\x1b\xb6\xee\x56\x00\x00\x00\x00IEND\xaeB`\x82' > "$IMG"

# 7a — raw octet-stream upload
upload=$(curl -s -X POST "$API/v1/assets/images/$DATASET?filename=test.png" \
  -H "Authorization: Bearer $TOKEN" \
  --data-binary "@$IMG")
echo "$upload" | grep -q '"assetId"' || fail "raw asset upload" "$upload"
ok "raw octet-stream upload → assetId"

ASSET_URL=$(echo "$upload" | python3 -c 'import json,sys;print(json.load(sys.stdin)["document"]["url"])')
code=$(curl -s -o /dev/null -w "%{http_code}" "$ASSET_URL")
[ "$code" = "200" ] || fail "serve asset" "$ASSET_URL → $code"
ok "GET $ASSET_URL served the stored file"

# 7b — multipart/form-data upload (Sanity's newer client path)
multi=$(curl -s -X POST "$API/v1/assets/images/$DATASET" \
  -H "Authorization: Bearer $TOKEN" \
  -F "file=@$IMG;filename=multi.png;type=image/png")
echo "$multi" | grep -q '"assetId"' || fail "multipart upload" "$multi"
ok "multipart/form-data upload → assetId"

# 7c — generic file endpoint
TXT=$(mktemp --suffix=.txt)
echo "hello self-hosted cms" > "$TXT"
fup=$(curl -s -X POST "$API/v1/assets/files/$DATASET?filename=note.txt" \
  -H "Authorization: Bearer $TOKEN" \
  --data-binary "@$TXT")
echo "$fup" | grep -q '"sanity.fileAsset"' || fail "file upload" "$fup"
ok "POST /v1/assets/files uploaded sanity.fileAsset"

rm -f "$IMG" "$TXT"

# ---- 8. presence WebSocket handshake ----
hdr "presence"
# Full WS testing requires a WS client; smoke-test just verifies the handshake
# upgrades (HTTP 101). Install `websocat` to exercise the full protocol.
code=$(curl -s -o /dev/null -w "%{http_code}" \
  -H "Connection: Upgrade" \
  -H "Upgrade: websocket" \
  -H "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==" \
  -H "Sec-WebSocket-Version: 13" \
  "$API/v1/presence/$DATASET")
[ "$code" = "101" ] || fail "presence upgrade" "got $code"
ok "/v1/presence/$DATASET upgrades to WebSocket (101)"

# ---- 9. delete ----
hdr "delete"
del=$(curl -s -X POST "$API/v1/data/mutate/$DATASET" \
  -H "Authorization: Bearer $TOKEN" \
  -H 'Content-Type: application/json' \
  -d "{\"mutations\":[{\"delete\":{\"id\":\"$DOC_ID\"}}]}")
echo "$del" | grep -q '"operation":"delete"' || fail "delete" "$del"
ok "deleted $DOC_ID"

after=$(curl -s "$API/v1/data/doc/$DATASET/$DOC_ID")
echo "$after" | grep -q '"omitted"' || fail "post-delete read" "$after"
ok "/v1/data/doc after delete returns it in 'omitted'"

# ---- summary ----
printf "\n\033[1;32mAll smoke tests passed.\033[0m\n"
