#!/usr/bin/env bash
# End-to-end: build hello-world → seed → host → invoke (cold + warm).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

PORT="${NITRUM_FN_E2E_PORT:-18090}"
DATA_DIR="${NITRUM_FN_E2E_DATA:-$ROOT/.data/e2e}"
SEED_DIR="$DATA_DIR/seed"
ARTIFACT_DIR="$DATA_DIR/artifacts"
HOST_LOG="$DATA_DIR/host.log"
HOST_PID=""

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
mkdir -p "$SEED_DIR" "$ARTIFACT_DIR"

echo "==> build + seed hello-world"
NITRUM_FN_SEED_DIR="$SEED_DIR" "$ROOT/examples/hello-world/deploy-local.sh"
[[ -f "$SEED_DIR/hello-world.wasm" ]] || fail "seed wasm missing"

echo "==> start host on :${PORT}"
NITRUM_FN_PORT="$PORT" \
NITRUM_FN_SEED_DIR="$SEED_DIR" \
NITRUM_FN_ARTIFACT_DIR="$ARTIFACT_DIR" \
  cargo run -p host >"$HOST_LOG" 2>&1 &
HOST_PID=$!

echo "==> wait for /healthz"
ready=0
for _ in $(seq 1 60); do
  if curl -sf "http://127.0.0.1:${PORT}/healthz" >/dev/null 2>&1; then
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

echo "==> cold invoke"
cold_headers="$(mktemp)"
cold_body="$(curl -sS -D "$cold_headers" -X POST \
  "http://127.0.0.1:${PORT}/invoke/hello-world" \
  -H 'content-type: application/json' \
  -d '{}')"

grep -qi '^HTTP/.* 200' "$cold_headers" || fail "cold status not 200 ($(head -1 "$cold_headers"))"
[[ "$cold_body" == '{"message":"Hello, world!"}' ]] || fail "cold body: $cold_body"
grep -qi '^x-nitrum-fn-warm: *0' "$cold_headers" || fail "expected x-nitrum-fn-warm: 0 on cold"
pass "cold invoke"

echo "==> warm invoke"
warm_headers="$(mktemp)"
warm_body="$(curl -sS -D "$warm_headers" -X POST \
  "http://127.0.0.1:${PORT}/invoke/hello-world" \
  -H 'content-type: application/json' \
  -d '{}')"

grep -qi '^HTTP/.* 200' "$warm_headers" || fail "warm status not 200 ($(head -1 "$warm_headers"))"
[[ "$warm_body" == '{"message":"Hello, world!"}' ]] || fail "warm body: $warm_body"
grep -qi '^x-nitrum-fn-warm: *1' "$warm_headers" || fail "expected x-nitrum-fn-warm: 1 on warm"
pass "warm invoke"

echo "==> unknown function → 404"
code="$(curl -sS -o /dev/null -w '%{http_code}' -X POST \
  "http://127.0.0.1:${PORT}/invoke/does-not-exist" \
  -H 'content-type: application/json' \
  -d '{}')"
[[ "$code" == "404" ]] || fail "expected 404 for missing fn, got $code"
pass "missing function 404"

echo
echo "e2e passed"
