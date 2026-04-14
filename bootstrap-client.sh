#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TEMPLATE_DIR="$ROOT_DIR/templates/starter"

APP_NAME=""
TARGET_DIR="$HOME/projects"
PROJECT_ID="default"
DATASET="production"
API_HOST="http://localhost:3030"
RUN_INSTALL=0

usage() {
  cat <<'EOF'
Bootstrap a new client Studio from templates/starter.

Usage:
  ./bootstrap-client.sh --name <app-name> [options]

Options:
  --name <name>          App folder name (required)
  --target <dir>         Parent directory where app folder is created (default: ~/projects)
  --project-id <id>      SANITY_STUDIO_PROJECT_ID (default: default)
  --dataset <name>       SANITY_STUDIO_DATASET (default: production)
  --api-host <url>       SANITY_STUDIO_API_HOST (default: http://localhost:3030)
  --install              Run pnpm install after scaffolding
  -h, --help             Show this help

Examples:
  ./bootstrap-client.sh --name client-xyz --install
  ./bootstrap-client.sh --name client-abc --target ~/work --project-id acme --dataset prod
EOF
}

sanitize_package_name() {
  local name="$1"
  echo "$name" | tr '[:upper:]' '[:lower:]' | tr ' _' '--' | tr -cd 'a-z0-9-'
}

set_env_var() {
  local file="$1"
  local key="$2"
  local value="$3"
  local tmp
  tmp="$(mktemp)"
  awk -v key="$key" -v value="$value" '
    BEGIN {found = 0}
    $0 ~ ("^" key "=") {print key "=" value; found = 1; next}
    {print}
    END {if (!found) print key "=" value}
  ' "$file" > "$tmp"
  mv "$tmp" "$file"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --name)
      APP_NAME="${2:-}"
      shift 2
      ;;
    --target)
      TARGET_DIR="${2:-}"
      shift 2
      ;;
    --project-id)
      PROJECT_ID="${2:-}"
      shift 2
      ;;
    --dataset)
      DATASET="${2:-}"
      shift 2
      ;;
    --api-host)
      API_HOST="${2:-}"
      shift 2
      ;;
    --install)
      RUN_INSTALL=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown option: $1" >&2
      usage
      exit 1
      ;;
  esac
done

if [[ -z "$APP_NAME" ]]; then
  echo "--name is required." >&2
  usage
  exit 1
fi

if [[ ! -d "$TEMPLATE_DIR" ]]; then
  echo "Template directory not found: $TEMPLATE_DIR" >&2
  exit 1
fi

mkdir -p "$TARGET_DIR"
APP_DIR="$TARGET_DIR/$APP_NAME"

if [[ -e "$APP_DIR" ]]; then
  echo "Target already exists: $APP_DIR" >&2
  exit 1
fi

cp -R "$TEMPLATE_DIR" "$APP_DIR"

if [[ -f "$APP_DIR/.env.example" ]]; then
  cp "$APP_DIR/.env.example" "$APP_DIR/.env"
  set_env_var "$APP_DIR/.env" "SANITY_STUDIO_PROJECT_ID" "$PROJECT_ID"
  set_env_var "$APP_DIR/.env" "SANITY_STUDIO_DATASET" "$DATASET"
  set_env_var "$APP_DIR/.env" "SANITY_STUDIO_API_HOST" "$API_HOST"
fi

if [[ -f "$APP_DIR/package.json" ]]; then
  PACKAGE_NAME="$(sanitize_package_name "$APP_NAME")"
  python3 - "$APP_DIR/package.json" "$PACKAGE_NAME" <<'PY'
import json
import sys
path = sys.argv[1]
name = sys.argv[2]
with open(path, "r", encoding="utf-8") as f:
    pkg = json.load(f)
pkg["name"] = name
with open(path, "w", encoding="utf-8") as f:
    json.dump(pkg, f, indent=2)
    f.write("\n")
PY
fi

echo "Created Studio app at: $APP_DIR"
echo "Configured .env:"
echo "  SANITY_STUDIO_PROJECT_ID=$PROJECT_ID"
echo "  SANITY_STUDIO_DATASET=$DATASET"
echo "  SANITY_STUDIO_API_HOST=$API_HOST"

if [[ "$RUN_INSTALL" -eq 1 ]]; then
  if command -v pnpm >/dev/null 2>&1; then
    (cd "$APP_DIR" && pnpm install)
  else
    echo "pnpm not found; skipping install."
  fi
fi

echo ""
echo "Next steps:"
echo "  cd \"$APP_DIR\""
echo "  pnpm dev"
