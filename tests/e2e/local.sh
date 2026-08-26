#!/usr/bin/env bash
# End-to-end: Floci (S3+SNS+SQS) + DynamoDB Local → api + host + publish-worker → CLI deploy → invoke.
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
FLOCI_URL="${NITRUM_FN_ARTIFACTS__ENDPOINT:-http://127.0.0.1:4566}"
DDB_URL="${NITRUM_FN_CATALOG__ENDPOINT:-http://127.0.0.1:8000}"
SQS_QUEUE_URL="${NITRUM_FN_COMPILE__QUEUE_URL:-${FLOCI_URL}/000000000000/nitrum-fn-compile}"
TOPIC_NAME="${NITRUM_FN_E2E_TOPIC:-nitrum-fn-publish}"
BUCKET="${NITRUM_FN_E2E_BUCKET:-nitrum-fn-e2e-$$}"
TABLE="${NITRUM_FN_E2E_TABLE:-nitrum-fn-e2e-$$}"
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
    NITRUM_FN_ARTIFACTS__BUCKET="$BUCKET" \
    NITRUM_FN_CATALOG__TABLE="$TABLE" \
    NITRUM_FN_CATALOG__IDEMPOTENCY_TABLE="${TABLE}-idempotency" \
    AWS_REGION=us-east-1 \
    AWS_DEFAULT_REGION=us-east-1 \
    AWS_ACCESS_KEY_ID=test \
    AWS_SECRET_ACCESS_KEY=test
}

provision_store() {
  command -v aws >/dev/null || fail "aws CLI is required to create Floci/DynamoDB Local resources"
  local queue_name
  queue_name="$(basename "${SQS_QUEUE_URL%/}")"

  aws --endpoint-url "$FLOCI_URL" s3 mb "s3://$BUCKET" >/dev/null

  aws --endpoint-url "$DDB_URL" dynamodb create-table \
    --table-name "$TABLE" \
    --attribute-definitions AttributeName=fn_id,AttributeType=S AttributeName=label,AttributeType=S \
    --key-schema AttributeName=fn_id,KeyType=HASH AttributeName=label,KeyType=RANGE \
    --billing-mode PAY_PER_REQUEST >/dev/null
  aws --endpoint-url "$DDB_URL" dynamodb wait table-exists --table-name "$TABLE"

  aws --endpoint-url "$DDB_URL" dynamodb create-table \
    --table-name "${TABLE}-idempotency" \
    --attribute-definitions AttributeName=idempotency_key,AttributeType=S \
    --key-schema AttributeName=idempotency_key,KeyType=HASH \
    --billing-mode PAY_PER_REQUEST >/dev/null
  aws --endpoint-url "$DDB_URL" dynamodb wait table-exists --table-name "${TABLE}-idempotency"
  aws --endpoint-url "$DDB_URL" dynamodb update-time-to-live \
    --table-name "${TABLE}-idempotency" \
    --time-to-live-specification Enabled=true,AttributeName=expires_at >/dev/null

  aws --endpoint-url "$FLOCI_URL" sqs create-queue \
    --queue-name "$queue_name" \
    --attributes VisibilityTimeout=300,ReceiveMessageWaitTimeSeconds=20 >/dev/null

  local queue_arn topic_arn
  queue_arn="$(aws --endpoint-url "$FLOCI_URL" sqs get-queue-attributes \
    --queue-url "$SQS_QUEUE_URL" \
    --attribute-names QueueArn \
    --query Attributes.QueueArn --output text)"
  topic_arn="$(aws --endpoint-url "$FLOCI_URL" sns create-topic \
    --name "$TOPIC_NAME" \
    --query TopicArn --output text)"
  aws --endpoint-url "$FLOCI_URL" sns subscribe \
    --topic-arn "$topic_arn" \
    --protocol sqs \
    --notification-endpoint "$queue_arn" \
    --attributes RawMessageDelivery=true >/dev/null
  export NITRUM_FN_PUBLISH__TOPIC_ARN="$topic_arn"
  export NITRUM_FN_PUBLISH__ENDPOINT="$FLOCI_URL"
}

echo "==> prepare data dir"
rm -rf "$DATA_DIR"
mkdir -p "$DATA_DIR"

echo "==> start Floci (S3+SNS+SQS) and DynamoDB Local"
docker compose up -d
COMPOSE_UP=1
wait_tcp 127.0.0.1 4566 "Floci"
wait_tcp 127.0.0.1 8000 "DynamoDB Local"
pass "emulators up"

common_env
echo "==> provision S3 bucket, DynamoDB tables, SNS topic, SQS queue"
provision_store
pass "store ready"

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
echo "e2e passed (Floci S3+SNS+SQS, DynamoDB Local, bucket=$BUCKET, table=$TABLE)"
