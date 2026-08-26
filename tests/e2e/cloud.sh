#!/usr/bin/env bash
# Cloud e2e: CLI deploy to the HTTP ALB, invoke through the NLB (self-signed enclave TLS).
# Requires a deployed staging stack with enable_enclave = true.
#
#   export NITRUM_FN_API_URL="$(terraform -chdir=infra/envs/staging output -raw api_url)"
#   export NITRUM_FN_INVOKE_URL="$(terraform -chdir=infra/envs/staging output -raw invoke_url)"
#   bash tests/e2e/cloud.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

API_URL="${NITRUM_FN_API_URL:-}"
INVOKE_URL="${NITRUM_FN_INVOKE_URL:-}"
EXAMPLE="$ROOT/examples/hello-world"
TARGET="wasm32-unknown-unknown"
WASM_SRC="$EXAMPLE/target/$TARGET/release/hello_world.wasm"
NAME="${NITRUM_FN_E2E_NAME:-hello-world}"
API_TIMEOUT="${NITRUM_FN_E2E_API_TIMEOUT:-90}"
INVOKE_TIMEOUT="${NITRUM_FN_E2E_INVOKE_TIMEOUT:-600}"

pass() { printf 'ok  - %s\n' "$*"; }
fail() { printf 'FAIL - %s\n' "$*" >&2; exit 1; }

if [[ -z "$API_URL" || -z "$INVOKE_URL" ]]; then
  fail "set NITRUM_FN_API_URL and NITRUM_FN_INVOKE_URL (terraform outputs api_url / invoke_url)"
fi

API_URL="${API_URL%/}"
INVOKE_URL="${INVOKE_URL%/}"

echo "==> build hello-world wasm"
rustup target add "$TARGET" >/dev/null
cargo build --manifest-path "$EXAMPLE/Cargo.toml" --target "$TARGET" --release
[[ -f "$WASM_SRC" ]] || fail "wasm missing at $WASM_SRC"
pass "wasm built"

echo "==> wait for API /healthz ($API_URL)"
ready=0
for _ in $(seq 1 "$API_TIMEOUT"); do
  if curl -sf "${API_URL}/healthz" >/dev/null 2>&1; then
    ready=1
    break
  fi
  sleep 1
done
[[ "$ready" -eq 1 ]] || fail "API healthz timeout. Check ECS service ${API_URL}"
pass "API healthy"

echo "==> CLI deploy"
cargo run -p cli --quiet -- deploy "$WASM_SRC" --name "$NAME" --url "$API_URL" \
  || fail "deploy failed (ALB=${API_URL})"
pass "deployed ${NAME}"

echo "==> GET /functions/${NAME}"
meta="$(curl -sf "${API_URL}/functions/${NAME}")" \
  || fail "GET function metadata"
echo "$meta" | grep -q "\"name\":\"${NAME}\"" || fail "metadata name: $meta"
pass "function metadata"

echo "==> wait for enclave (self-signed TLS on NLB, up to ${INVOKE_TIMEOUT}s)"
ready=0
for _ in $(seq 1 "$INVOKE_TIMEOUT"); do
  if curl -skf "${INVOKE_URL}/.well-known/enclave/status" >/dev/null 2>&1 \
    || curl -skf "${INVOKE_URL}/healthz" >/dev/null 2>&1; then
    ready=1
    break
  fi
  sleep 1
done
[[ "$ready" -eq 1 ]] || fail "enclave not ready at ${INVOKE_URL}. Check ASG / control-plane logs; NLB DNS=${INVOKE_URL}"
pass "enclave reachable"

echo "==> invoke after deploy"
headers="$(mktemp)"
body="$(curl -skS -D "$headers" -X POST \
  "${INVOKE_URL}/invoke/${NAME}" \
  -H 'content-type: application/json' \
  -d '{}')"

grep -qi '^HTTP/.* 200' "$headers" || fail "status not 200 ($(head -1 "$headers")); body=${body}"
[[ "$body" == '{"message":"Hello, world!"}' ]] || fail "body: $body"
pass "invoke after deploy"

echo "==> unknown function → 404"
code="$(curl -skS -o /dev/null -w '%{http_code}' -X POST \
  "${INVOKE_URL}/invoke/does-not-exist" \
  -H 'content-type: application/json' \
  -d '{}')"
[[ "$code" == "404" ]] || fail "expected 404 for missing fn, got $code"
pass "missing function 404"

echo
echo "cloud e2e passed (api=${API_URL}, invoke=${INVOKE_URL})"
