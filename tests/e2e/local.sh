#!/usr/bin/env bash
# End-to-end: Floci → api + host → nitrum-fn new → build → account → deploy → invoke.
#
# Local YAML leaves billing at zero. This script turns it on (cost 1) and funds
# the account with `account credit`. The mock facilitator (tests/mocks/facilitator.sh)
# accepts the signed x402 retry, so no chain or funds are involved.
#
# Environment (optional; `set -a && source .env && set +a` — see .env.example):
#   NITRUM_FN_BIN / NITRUM_FN_E2E_RUST_LOG
#   NITRUM_FN_E2E_PARENT_DIR / NITRUM_FN_E2E_INIT_NAME
#   NITRUM_FN_E2E_API_PORT / NITRUM_FN_E2E_HOST_PORT / NITRUM_FN_E2E_DATA
#
# Usage: from repo root, `bash tests/e2e/local.sh`

set -euo pipefail

E2E_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${E2E_DIR}/../.." && pwd)"
cd "${REPO_ROOT}"

# shellcheck source=lib.sh
source "${E2E_DIR}/lib.sh"

# --- constants ---
API_PORT="${NITRUM_FN_E2E_API_PORT:-18090}"
HOST_PORT="${NITRUM_FN_E2E_HOST_PORT:-18091}"
DATA_DIR="${NITRUM_FN_E2E_DATA:-${REPO_ROOT}/.data/e2e}"
API_LOG="${DATA_DIR}/api.log"
HOST_LOG="${DATA_DIR}/host.log"
FACILITATOR_LOG="${DATA_DIR}/facilitator.log"
API_PID=""
HOST_PID=""
FACILITATOR_PID=""
COMPOSE_UP=0
API_URL="http://127.0.0.1:${API_PORT}"
HOST_URL="http://127.0.0.1:${HOST_PORT}"
# Port that config/shared/local.yaml points billing.facilitator_url at.
FACILITATOR_URL="http://127.0.0.1:4402"
EXPECTED_BODY='{"message":"Hello, world!"}'
NITRUM_FN_ENV=local
AWS_REGION=us-east-1
AWS_DEFAULT_REGION=us-east-1
AWS_ACCESS_KEY_ID=test
AWS_SECRET_ACCESS_KEY=test
AWS_EC2_METADATA_DISABLED=true
# Public test key (Hardhat account 0). The mock facilitator does not check it.
PAYER_KEY=0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80
DEPLOY_MIN=1
INVOKE_COST=1
GRANT_INVOKES=2
# billing.usdc_per_invoke in config/shared/local.yaml (atomic USDC).
USDC_PER_INVOKE=10000

e2e_init_workspace

# --- required ---
require_cmd docker
require_cmd cargo
require_cmd curl
require_cmd nc
require_cmd rustup

log_banner "nitrum-fn  local e2e"
log_step "parameters"
log_kv "name" "${NAME}"
log_kv "parent" "${PARENT}"
log_kv "api" "${API_URL}"
log_kv "host" "${HOST_URL}"
log_kv "data" "${DATA_DIR}"
log_kv "cli" "$(cli_display)"

cleanup() {
    for pid in "${API_PID}" "${HOST_PID}" "${FACILITATOR_PID}"; do
        if [[ -n "${pid}" ]] && kill -0 "${pid}" 2>/dev/null; then
            kill "${pid}" 2>/dev/null || true
            wait "${pid}" 2>/dev/null || true
        fi
    done
    if [[ "${COMPOSE_UP}" -eq 1 ]]; then
        docker compose down -v --remove-orphans >/dev/null 2>&1 || true
    fi
}
trap cleanup EXIT

dump_logs() {
    log_step "api.log (tail)"
    if [[ -f "${API_LOG}" ]]; then
        tail -n 80 "${API_LOG}" >&2 || true
    else
        log_note "(missing ${API_LOG})"
    fi
    log_step "facilitator.log (tail)"
    if [[ -f "${FACILITATOR_LOG}" ]]; then
        tail -n 20 "${FACILITATOR_LOG}" >&2 || true
    fi
    log_step "host.log (tail)"
    if [[ -f "${HOST_LOG}" ]]; then
        tail -n 80 "${HOST_LOG}" >&2 || true
    else
        log_note "(missing ${HOST_LOG})"
    fi
}

wait_healthz() {
    local url="$1" pid="$2" log="$3" label="$4"
    local ready=0
    # 10 min: cold CI still compiles wasmtime even after a workspace cargo build.
    for _ in $(seq 1 1200); do
        if curl -sf "${url}/healthz" >/dev/null 2>&1; then
            ready=1
            break
        fi
        if ! kill -0 "${pid}" 2>/dev/null; then
            dump_logs
            die "${label} exited before becoming ready"
        fi
        sleep 0.5
    done
    [[ "${ready}" -eq 1 ]] || { dump_logs; die "${label} healthz timeout"; }
}

common_env() {
    export NITRUM_FN_ENV AWS_REGION AWS_DEFAULT_REGION AWS_ACCESS_KEY_ID AWS_SECRET_ACCESS_KEY
    export AWS_EC2_METADATA_DISABLED
    # A developer shell may export these. PCR0 makes invoke require attestation.
    unset NITRUM_FN_PCR0 NITRUM_FN_API_KEY NITRUM_FN_PRIVATE_KEY
}

log_step "prepare data dir"
rm -rf "${DATA_DIR}"
mkdir -p "${DATA_DIR}"

log_step "start Floci and provision store"
COMPOSE_UP=1
if ! docker compose up -d --remove-orphans floci; then
    docker compose logs >&2 || true
    die "Floci failed to start"
fi
if ! docker compose run --rm aws-init; then
    docker compose logs >&2 || true
    die "aws-init (Floci seed) failed"
fi
log_ok "store ready"
common_env

# Compile once so the two `cargo run`s do not fight the target lock on a cold CI cache.
log_step "build api, host, and CLI"
if ! cargo build -p api -p host -p cli; then
    die "workspace build failed"
fi
log_ok "binaries ready"

log_step "start mock facilitator on ${FACILITATOR_URL}"
bash "${REPO_ROOT}/tests/mocks/facilitator.sh" >"${FACILITATOR_LOG}" 2>&1 &
FACILITATOR_PID=$!

log_step "start api on :${API_PORT} (publish minimum ${DEPLOY_MIN})"
NITRUM_FN_SERVER__PORT="${API_PORT}" \
NITRUM_FN_BILLING__MIN_DEPLOY_CREDITS="${DEPLOY_MIN}" \
    cargo run -p api >"${API_LOG}" 2>&1 &
API_PID=$!

log_step "start host on :${HOST_PORT} (invoke cost ${INVOKE_COST})"
NITRUM_FN_SERVER__PORT="${HOST_PORT}" \
NITRUM_FN_BILLING__INVOKE_CREDIT_COST="${INVOKE_COST}" \
    cargo run -p host >"${HOST_LOG}" 2>&1 &
HOST_PID=$!

log_step "wait for /healthz"
wait_healthz "${API_URL}" "${API_PID}" "${API_LOG}" "api"
log_ok "api healthy"
wait_healthz "${HOST_URL}" "${HOST_PID}" "${HOST_LOG}" "host"
log_ok "host healthy"
wait_healthz "${FACILITATOR_URL}" "${FACILITATOR_PID}" "${FACILITATOR_LOG}" "facilitator"
log_ok "facilitator healthy"

step_scaffold_and_build

http_code() {
    local body_file="$1"
    shift
    curl -sS -o "${body_file}" -w '%{http_code}' "$@"
}

expect_status() {
    local want="$1"
    local body_file="${DATA_DIR}/http.body"
    shift
    local code
    code="$(http_code "${body_file}" "$@")" || { dump_logs; die "curl failed"; }
    [[ "${code}" == "${want}" ]] || {
        dump_logs
        die "expected HTTP ${want}, got ${code}: $(cat "${body_file}")"
    }
}

account_balance() {
    local body
    body="$(curl -sf "${API_URL}/accounts/${ACCOUNT_ID}" \
        -H "authorization: Bearer ${SECRET}")" \
        || { dump_logs; die "GET account failed"; }
    printf '%s' "${body}" | sed -n 's/.*"balance":\([0-9]*\).*/\1/p'
}

log_step "account create"
create_out="$(cli account create --url "${API_URL}")" \
    || { dump_logs; die "account create failed"; }
ACCOUNT_ID="$(printf '%s\n' "${create_out}" | awk -F= '/^account_id=/{print $2}')"
SECRET="$(printf '%s\n' "${create_out}" | awk -F= '/^secret=/{print $2}')"
[[ -n "${ACCOUNT_ID}" && -n "${SECRET}" ]] || die "account create output: ${create_out}"
log_ok "account ${ACCOUNT_ID}"

log_step "deploy without a bearer → 401"
before="$(curl -sS -w '\n%{http_code}' "${API_URL}/functions/${NAME}" || true)"
expect_status 401 -X PUT --data-binary @"${WASM_SRC}" \
    -H 'content-type: application/wasm' \
    "${API_URL}/functions/${NAME}"
after="$(curl -sS -w '\n%{http_code}' "${API_URL}/functions/${NAME}" || true)"
[[ "${before}" == "${after}" ]] || die "unauthenticated deploy changed the catalog"
log_ok "deploy rejected"

log_step "invoke without a bearer → 401"
expect_status 401 -X POST \
    -H 'content-type: application/json' \
    -d '{}' \
    "${HOST_URL}/invoke/${NAME}"
log_ok "invoke rejected"

log_step "credit without a payment key → refused, balance unchanged"
if cli account credit --account "${ACCOUNT_ID}" --invokes "${GRANT_INVOKES}" \
    --url "${API_URL}" >/dev/null 2>&1; then
    die "credit succeeded without a signed payment"
fi
[[ "$(account_balance)" == "0" ]] || die "unpaid credit changed the balance"
log_ok "unpaid credit rejected"

log_step "credit above --max-usdc → refused before signing, balance unchanged"
if NITRUM_FN_PRIVATE_KEY="${PAYER_KEY}" \
    cli account credit --account "${ACCOUNT_ID}" --invokes "${GRANT_INVOKES}" \
    --url "${API_URL}" --max-usdc 1 >/dev/null 2>&1; then
    die "credit signed a price above --max-usdc"
fi
[[ "$(account_balance)" == "0" ]] || die "capped credit changed the balance"
log_ok "price cap enforced"

log_step "account credit ${GRANT_INVOKES} invokes (x402 via mock facilitator)"
NITRUM_FN_PRIVATE_KEY="${PAYER_KEY}" \
    cli account credit --account "${ACCOUNT_ID}" --invokes "${GRANT_INVOKES}" --url "${API_URL}" \
    --max-usdc "$((GRANT_INVOKES * USDC_PER_INVOKE))" \
    || { dump_logs; die "account credit failed"; }
[[ "$(account_balance)" == "${GRANT_INVOKES}" ]] || die "credit did not add ${GRANT_INVOKES} invokes"
log_ok "funded"

log_step "CLI deploy ${NAME}"
cli deploy "${WASM_SRC}" --name "${NAME}" --url "${API_URL}" --api-key "${SECRET}" \
    || { dump_logs; die "deploy failed"; }
balance="$(account_balance)"
[[ "${balance}" == "${GRANT_INVOKES}" ]] || die "deploy debited the balance (got ${balance})"
log_ok "deployed ${NAME} (balance still ${balance})"

log_step "GET /functions/${NAME}"
meta="$(curl -sf "${API_URL}/functions/${NAME}")" \
    || { dump_logs; die "GET function metadata"; }
echo "${meta}" | grep -q "\"name\":\"${NAME}\"" || die "metadata name: ${meta}"
log_ok "function metadata"

log_step "CLI invoke --fn-shasum"
err="${DATA_DIR}/invoke.err"
body="$(cli invoke "${NAME}" --url "${HOST_URL}" -d '{}' \
    --fn-shasum "${HASH}" --api-key "${SECRET}" 2>"${err}")" \
    || { dump_logs; cat "${err}" >&2 || true; die "invoke failed"; }

[[ "${body}" == "${EXPECTED_BODY}" ]] || die "body: ${body}"
balance="$(account_balance)"
expected_after="$((GRANT_INVOKES - INVOKE_COST))"
[[ "${balance}" == "${expected_after}" ]] || die "invoke balance: got ${balance}, want ${expected_after}"
log_ok "balance ${balance} after invoke"
grep -q "x-nitrum-fn-shasum=${HASH}" "${err}" \
    || die "missing/mismatched shasum header (expected ${HASH}): $(cat "${err}")"
while IFS= read -r line; do
    [[ -n "${line}" ]] && log_note "${line}"
done < "${err}"
log_ok "invoke after deploy (hash verified)"

log_step "unknown function → 404 and refund"
code="$(curl -sS -o /dev/null -w '%{http_code}' -X POST \
    "${HOST_URL}/invoke/does-not-exist" \
    -H 'content-type: application/json' \
    -H "authorization: Bearer ${SECRET}" \
    -d '{}')"
[[ "${code}" == "404" ]] || die "expected 404 for missing fn, got ${code}"
balance="$(account_balance)"
[[ "${balance}" == "${expected_after}" ]] || die "missing function kept the debit (balance ${balance})"
log_ok "missing function 404, balance still ${balance}"

dump_logs

log_pass "local e2e passed" \
    store "Floci S3+DynamoDB" \
    api "${API_URL}" \
    host "${HOST_URL}" \
    project "${PROJECT}"
