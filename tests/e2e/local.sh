#!/usr/bin/env bash
# End-to-end: Floci (S3+DynamoDB) → api + host → CLI deploy → invoke.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

API_PORT="${NITRUM_FN_E2E_API_PORT:-18090}"
HOST_PORT="${NITRUM_FN_E2E_HOST_PORT:-18091}"
DATA_DIR="${NITRUM_FN_E2E_DATA:-$ROOT/.data/e2e}"
API_LOG="$DATA_DIR/api.log"
HOST_LOG="$DATA_DIR/host.log"
API_PID=""
HOST_PID=""
COMPOSE_UP=0
API_URL="http://127.0.0.1:${API_PORT}"
HOST_URL="http://127.0.0.1:${HOST_PORT}"
EXAMPLE="$ROOT/examples/hello-world"
TARGET="wasm32-unknown-unknown"
WASM_SRC="$EXAMPLE/target/$TARGET/release/hello_world.wasm"

cleanup() {
  for pid in "${API_PID}" "${HOST_PID}"; do
    if [[ -n "${pid}" ]] && kill -0 "${pid}" 2>/dev/null; then
      kill "${pid}" 2>/dev/null || true
      wait "${pid}" 2>/dev/null || true
    fi
  done
  if [[ "$COMPOSE_UP" -eq 1 ]]; then
    docker compose down -v --remove-orphans >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

pass() { printf 'ok  - %s\n' "$*"; }
fail() { printf 'FAIL - %s\n' "$*" >&2; exit 1; }

dump_logs() {
  cat "$API_LOG" >&2 || true
  cat "$HOST_LOG" >&2 || true
}

wait_healthz() {
  local url="$1" pid="$2" log="$3" label="$4"
  local ready=0
  for _ in $(seq 1 180); do
    if curl -sf "${url}/healthz" >/dev/null 2>&1; then
      ready=1
      break
    fi
    if ! kill -0 "${pid}" 2>/dev/null; then
      cat "$log" >&2 || true
      fail "$label exited before becoming ready"
    fi
    sleep 0.5
  done
  [[ "$ready" -eq 1 ]] || { cat "$log" >&2 || true; fail "$label healthz timeout"; }
}

common_env() {
  export NITRUM_FN_ENV=local \
    AWS_REGION=us-east-1 \
    AWS_DEFAULT_REGION=us-east-1 \
    AWS_ACCESS_KEY_ID=test \
    AWS_SECRET_ACCESS_KEY=test
}

echo "==> prepare data dir"
rm -rf "$DATA_DIR"
mkdir -p "$DATA_DIR"

echo "==> start Floci and provision store"
COMPOSE_UP=1
if ! docker compose up -d --remove-orphans floci; then
  docker compose logs >&2 || true
  fail "Floci failed to start"
fi
if ! docker compose run --rm aws-init; then
  docker compose logs >&2 || true
  fail "aws-init (Floci seed) failed"
fi
pass "store ready"
common_env

echo "==> start api on :${API_PORT} (publish + catalog)"
NITRUM_FN_SERVER__PORT="$API_PORT" \
  cargo run -p api >"$API_LOG" 2>&1 &
API_PID=$!

echo "==> start host on :${HOST_PORT} (invoke)"
NITRUM_FN_SERVER__PORT="$HOST_PORT" \
  cargo run -p host >"$HOST_LOG" 2>&1 &
HOST_PID=$!

echo "==> wait for /healthz"
wait_healthz "$API_URL" "$API_PID" "$API_LOG" "api"
pass "api healthy"
wait_healthz "$HOST_URL" "$HOST_PID" "$HOST_LOG" "host"
pass "host healthy"

echo "==> build hello-world wasm"
rustup target add "$TARGET" >/dev/null
cargo build --manifest-path "$EXAMPLE/Cargo.toml" --target "$TARGET" --release
[[ -f "$WASM_SRC" ]] || fail "wasm missing at $WASM_SRC"
pass "wasm built"

echo "==> CLI deploy (validate + store + catalog upsert)"
cargo run -p cli --quiet -- deploy "$WASM_SRC" --name hello-world --url "$API_URL" \
  || { dump_logs; fail "deploy failed"; }
pass "deployed hello-world"

echo "==> GET /functions/hello-world"
meta="$(curl -sf "${API_URL}/functions/hello-world")" \
  || { dump_logs; fail "GET function metadata"; }
echo "$meta" | grep -q '"name":"hello-world"' || fail "metadata name: $meta"
pass "function metadata"

echo "==> CLI describe"
EXPECTED_HASH="$(shasum -a 256 "$WASM_SRC" | awk '{print $1}')"
desc="$(cargo run -p cli --quiet -- describe "$WASM_SRC")" \
  || { dump_logs; fail "describe failed"; }
echo "$desc" | grep -q "^hash=${EXPECTED_HASH}$" \
  || fail "describe hash mismatch (expected ${EXPECTED_HASH}): ${desc}"
pass "describe (content hash)"

echo "==> invoke after deploy (verify x-nitrum-fn-shasum)"
body="$(cargo run -p cli --quiet -- invoke hello-world --url "$HOST_URL" -d '{}' \
  --fn-shasum "$EXPECTED_HASH" 2>"$DATA_DIR/invoke.err")" \
  || { dump_logs; cat "$DATA_DIR/invoke.err" >&2 || true; fail "invoke failed"; }

[[ "$body" == '{"message":"Hello, world!"}' ]] || fail "body: $body"
grep -q "x-nitrum-fn-shasum=${EXPECTED_HASH}" "$DATA_DIR/invoke.err" \
  || fail "missing/mismatched shasum header (expected ${EXPECTED_HASH}): $(cat "$DATA_DIR/invoke.err")"
pass "invoke after deploy (hash verified)"

echo "==> unknown function → 404"
code="$(curl -sS -o /dev/null -w '%{http_code}' -X POST \
  "${HOST_URL}/invoke/does-not-exist" \
  -H 'content-type: application/json' \
  -d '{}')"
[[ "$code" == "404" ]] || fail "expected 404 for missing fn, got $code"
pass "missing function 404"

echo
echo "e2e passed (Floci S3+DynamoDB)"
