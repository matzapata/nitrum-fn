#!/usr/bin/env bash
# End-to-end: Floci (S3+SQS) + DynamoDB Local → host + publish-worker → CLI publish → invoke.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

PORT="${NITRUM_FN_E2E_PORT:-18090}"
DATA_DIR="${NITRUM_FN_E2E_DATA:-$ROOT/.data/e2e}"
HOST_LOG="$DATA_DIR/host.log"
WORKER_LOG="$DATA_DIR/worker.log"
HOST_PID=""
WORKER_PID=""
COMPOSE_UP=0
URL="http://127.0.0.1:${PORT}"
FLOCI_URL="${NITRUM_FN_S3_ENDPOINT:-http://127.0.0.1:4566}"
DDB_URL="${NITRUM_FN_DDB_ENDPOINT:-http://127.0.0.1:8000}"
# Floci serves SQS on the same endpoint as S3.
SQS_ENDPOINT="${NITRUM_FN_SQS_ENDPOINT:-$FLOCI_URL}"
SQS_QUEUE_URL="${NITRUM_FN_SQS_QUEUE_URL:-${FLOCI_URL}/000000000000/nitrum-fn-compile}"
BUCKET="${NITRUM_FN_E2E_BUCKET:-nitrum-fn-e2e-$$}"
TABLE="${NITRUM_FN_E2E_TABLE:-nitrum-fn-e2e-$$}"
EXAMPLE="$ROOT/examples/hello-world"
TARGET="wasm32-unknown-unknown"
WASM_SRC="$EXAMPLE/target/$TARGET/release/hello_world.wasm"

cleanup() {
  if [[ -n "${HOST_PID}" ]] && kill -0 "${HOST_PID}" 2>/dev/null; then
    kill "${HOST_PID}" 2>/dev/null || true
    wait "${HOST_PID}" 2>/dev/null || true
  fi
  if [[ -n "${WORKER_PID}" ]] && kill -0 "${WORKER_PID}" 2>/dev/null; then
    kill "${WORKER_PID}" 2>/dev/null || true
    wait "${WORKER_PID}" 2>/dev/null || true
  fi
  if [[ "$COMPOSE_UP" -eq 1 ]]; then
    docker compose down >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

pass() { printf 'ok  - %s\n' "$*"; }
fail() { printf 'FAIL - %s\n' "$*" >&2; exit 1; }

wait_tcp() {
  local host="$1" port="$2" label="$3"
  local ready=0
  for _ in $(seq 1 60); do
    if (echo >/dev/tcp/"$host"/"$port") >/dev/null 2>&1; then
      ready=1
      break
    fi
    sleep 0.5
  done
  [[ "$ready" -eq 1 ]] || fail "$label not reachable on ${host}:${port}"
}

common_env() {
  export NITRUM_FN_STORE=aws \
    NITRUM_FN_S3_BUCKET="$BUCKET" \
    NITRUM_FN_S3_ENDPOINT="$FLOCI_URL" \
    NITRUM_FN_S3_CREATE_BUCKET=true \
    NITRUM_FN_DDB_TABLE="$TABLE" \
    NITRUM_FN_DDB_ENDPOINT="$DDB_URL" \
    NITRUM_FN_DDB_CREATE_TABLE=true \
    NITRUM_FN_SQS_QUEUE_URL="$SQS_QUEUE_URL" \
    NITRUM_FN_SQS_ENDPOINT="$SQS_ENDPOINT" \
    NITRUM_FN_SQS_CREATE_QUEUE=true \
    AWS_REGION=us-east-1 \
    AWS_DEFAULT_REGION=us-east-1 \
    AWS_ACCESS_KEY_ID=test \
    AWS_SECRET_ACCESS_KEY=test
}

echo "==> prepare data dir"
rm -rf "$DATA_DIR"
mkdir -p "$DATA_DIR"

echo "==> start Floci (S3+SQS) and DynamoDB Local"
docker compose up -d
COMPOSE_UP=1
wait_tcp 127.0.0.1 4566 "Floci"
wait_tcp 127.0.0.1 8000 "DynamoDB Local"
pass "emulators up"

common_env

echo "==> start publish-worker (SQS → AOT)"
cargo run -p publish-worker >"$WORKER_LOG" 2>&1 &
WORKER_PID=$!

echo "==> start host on :${PORT} (S3 artifacts, DynamoDB catalog, Floci SQS publish)"
NITRUM_FN_PORT="$PORT" \
NITRUM_FN_SEED_DIR="$DATA_DIR/seed-empty" \
  cargo run -p host >"$HOST_LOG" 2>&1 &
HOST_PID=$!

echo "==> wait for /healthz"
ready=0
for _ in $(seq 1 180); do
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

echo "==> CLI publish (queues AOT; polls until ready)"
cargo run -p cli --quiet -- publish "$WASM_SRC" --name hello-world --url "$URL" \
  || { cat "$HOST_LOG" >&2 || true; cat "$WORKER_LOG" >&2 || true; fail "publish failed"; }
pass "published hello-world"

echo "==> GET /functions/hello-world"
meta="$(curl -sf "${URL}/functions/hello-world")" \
  || { cat "$HOST_LOG" >&2 || true; cat "$WORKER_LOG" >&2 || true; fail "GET function metadata"; }
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
echo "e2e passed (Floci S3+SQS, DynamoDB Local, bucket=$BUCKET, table=$TABLE)"
