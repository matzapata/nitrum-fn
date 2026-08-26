#!/usr/bin/env bash
# End-to-end: Floci (S3+SNS+SQS+DynamoDB) → api + host + publish-worker → CLI deploy → invoke.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

API_PORT="${NITRUM_FN_E2E_API_PORT:-18090}"
HOST_PORT="${NITRUM_FN_E2E_HOST_PORT:-18091}"
DATA_DIR="${NITRUM_FN_E2E_DATA:-$ROOT/.data/e2e}"
API_LOG="$DATA_DIR/api.log"
HOST_LOG="$DATA_DIR/host.log"
WORKER_LOG="$DATA_DIR/worker.log"
API_PID=""
HOST_PID=""
WORKER_PID=""
COMPOSE_UP=0
API_URL="http://127.0.0.1:${API_PORT}"
HOST_URL="http://127.0.0.1:${HOST_PORT}"
EXAMPLE="$ROOT/examples/hello-world"
TARGET="wasm32-unknown-unknown"
WASM_SRC="$EXAMPLE/target/$TARGET/release/hello_world.wasm"

cleanup() {
  for pid in "${API_PID}" "${HOST_PID}" "${WORKER_PID}"; do
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
  cat "$WORKER_LOG" >&2 || true
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

echo "==> start publish-worker (SQS → AOT)"
cargo run -p publish-worker >"$WORKER_LOG" 2>&1 &
WORKER_PID=$!

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

if ! kill -0 "${WORKER_PID}" 2>/dev/null; then
  cat "$WORKER_LOG" >&2 || true
  fail "publish-worker exited early"
fi
pass "publish-worker running"

echo "==> build hello-world wasm"
rustup target add "$TARGET" >/dev/null
cargo build --manifest-path "$EXAMPLE/Cargo.toml" --target "$TARGET" --release
[[ -f "$WASM_SRC" ]] || fail "wasm missing at $WASM_SRC"
pass "wasm built"

echo "==> CLI deploy (queues AOT; polls until ready)"
cargo run -p cli --quiet -- deploy "$WASM_SRC" --name hello-world --url "$API_URL" \
  || { dump_logs; fail "deploy failed"; }
pass "deployed hello-world"

echo "==> GET /functions/hello-world"
meta="$(curl -sf "${API_URL}/functions/hello-world")" \
  || { dump_logs; fail "GET function metadata"; }
echo "$meta" | grep -q '"name":"hello-world"' || fail "metadata name: $meta"
pass "function metadata"

echo "==> invoke after deploy"
headers="$(mktemp)"
body="$(curl -sS -D "$headers" -X POST \
  "${HOST_URL}/invoke/hello-world" \
  -H 'content-type: application/json' \
  -d '{}')"

grep -qi '^HTTP/.* 200' "$headers" || fail "status not 200 ($(head -1 "$headers"))"
[[ "$body" == '{"message":"Hello, world!"}' ]] || fail "body: $body"
pass "invoke after deploy"

echo "==> unknown function → 404"
code="$(curl -sS -o /dev/null -w '%{http_code}' -X POST \
  "${HOST_URL}/invoke/does-not-exist" \
  -H 'content-type: application/json' \
  -d '{}')"
[[ "$code" == "404" ]] || fail "expected 404 for missing fn, got $code"
pass "missing function 404"

echo
echo "e2e passed (Floci S3+SNS+SQS+DynamoDB)"
