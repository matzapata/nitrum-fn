#!/usr/bin/env bash
# End-to-end: start host → build hello-world → CLI publish → invoke (warm after deploy).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

PORT="${NITRUM_FN_E2E_PORT:-18090}"
DATA_DIR="${NITRUM_FN_E2E_DATA:-$ROOT/.data/e2e}"
ARTIFACT_DIR="$DATA_DIR/artifacts"
CATALOG_PATH="$DATA_DIR/catalog.json"
HOST_LOG="$DATA_DIR/host.log"
HOST_PID=""
URL="http://127.0.0.1:${PORT}"
EXAMPLE="$ROOT/examples/hello-world"
TARGET="wasm32-unknown-unknown"
WASM_SRC="$EXAMPLE/target/$TARGET/release/hello_world.wasm"

cleanup() {
  if [[ -n "${HOST_PID}" ]] && kill -0 "${HOST_PID}" 2>/dev/null; then
    kill "${HOST_PID}" 2>/dev/null || true
    wait "${HOST_PID}" 2>/dev/null || true
  fi
}
trap cleanup EXIT

pass() { printf 'ok  - %s\n' "$*"; }
fail() { printf 'FAIL - %s\n' "$*" >&2; exit 1; }

echo "==> prepare data dir"
rm -rf "$DATA_DIR"
mkdir -p "$ARTIFACT_DIR"

echo "==> start host on :${PORT}"
NITRUM_FN_PORT="$PORT" \
NITRUM_FN_ARTIFACT_DIR="$ARTIFACT_DIR" \
NITRUM_FN_CATALOG_PATH="$CATALOG_PATH" \
NITRUM_FN_SEED_DIR="$DATA_DIR/seed-empty" \
  cargo run -p host >"$HOST_LOG" 2>&1 &
HOST_PID=$!

echo "==> wait for /healthz"
ready=0
for _ in $(seq 1 60); do
  if curl -sf "${URL}/healthz" >/dev/null 2>&1; then
    ready=1
    break
  fi
  if ! kill -0 "${HOST_PID}" 2>/dev/null; then
    cat "$HOST_LOG" >&2 || true
    fail "host exited before becoming ready"
  fi
  sleep 0.5
done
[[ "$ready" -eq 1 ]] || { cat "$HOST_LOG" >&2 || true; fail "healthz timeout"; }
pass "host healthy"

echo "==> build hello-world wasm"
rustup target add "$TARGET" >/dev/null
cargo build --manifest-path "$EXAMPLE/Cargo.toml" --target "$TARGET" --release
[[ -f "$WASM_SRC" ]] || fail "wasm missing at $WASM_SRC"
pass "wasm built"

echo "==> CLI publish"
cargo run -p cli --quiet -- publish "$WASM_SRC" --name hello-world --url "$URL" \
  || { cat "$HOST_LOG" >&2 || true; fail "publish failed"; }
pass "published hello-world"

echo "==> GET /functions/hello-world"
meta="$(curl -sf "${URL}/functions/hello-world")" \
  || { cat "$HOST_LOG" >&2 || true; fail "GET function metadata"; }
echo "$meta" | grep -q '"name":"hello-world"' || fail "metadata name: $meta"
pass "function metadata"

echo "==> invoke after publish"
headers="$(mktemp)"
body="$(curl -sS -D "$headers" -X POST \
  "${URL}/invoke/hello-world" \
  -H 'content-type: application/json' \
  -d '{}')"

grep -qi '^HTTP/.* 200' "$headers" || fail "status not 200 ($(head -1 "$headers"))"
[[ "$body" == '{"message":"Hello, world!"}' ]] || fail "body: $body"
pass "invoke after publish"

echo "==> unknown function → 404"
code="$(curl -sS -o /dev/null -w '%{http_code}' -X POST \
  "${URL}/invoke/does-not-exist" \
  -H 'content-type: application/json' \
  -d '{}')"
[[ "$code" == "404" ]] || fail "expected 404 for missing fn, got $code"
pass "missing function 404"

echo
echo "e2e passed"
