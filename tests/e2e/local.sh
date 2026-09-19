#!/usr/bin/env bash
# End-to-end: Floci → api + host → nitrum-fn new → build → deploy → invoke.
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
API_PID=""
HOST_PID=""
COMPOSE_UP=0
API_URL="http://127.0.0.1:${API_PORT}"
HOST_URL="http://127.0.0.1:${HOST_PORT}"
EXPECTED_BODY='{"message":"Hello, world!"}'
NITRUM_FN_ENV=local
AWS_REGION=us-east-1
AWS_DEFAULT_REGION=us-east-1
AWS_ACCESS_KEY_ID=test
AWS_SECRET_ACCESS_KEY=test

e2e_init_workspace

# --- required ---
require_cmd docker
require_cmd cargo
require_cmd curl
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
    for pid in "${API_PID}" "${HOST_PID}"; do
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

log_step "start api on :${API_PORT} (publish + catalog)"
NITRUM_FN_SERVER__PORT="${API_PORT}" \
    cargo run -p api >"${API_LOG}" 2>&1 &
API_PID=$!

log_step "start host on :${HOST_PORT} (invoke)"
NITRUM_FN_SERVER__PORT="${HOST_PORT}" \
    cargo run -p host >"${HOST_LOG}" 2>&1 &
HOST_PID=$!

log_step "wait for /healthz"
wait_healthz "${API_URL}" "${API_PID}" "${API_LOG}" "api"
log_ok "api healthy"
wait_healthz "${HOST_URL}" "${HOST_PID}" "${HOST_LOG}" "host"
log_ok "host healthy"

step_scaffold_and_build

log_step "CLI deploy ${NAME}"
cli deploy "${WASM_SRC}" --name "${NAME}" --url "${API_URL}" \
    || { dump_logs; die "deploy failed"; }
log_ok "deployed ${NAME}"

log_step "GET /functions/${NAME}"
meta="$(curl -sf "${API_URL}/functions/${NAME}")" \
    || { dump_logs; die "GET function metadata"; }
echo "${meta}" | grep -q "\"name\":\"${NAME}\"" || die "metadata name: ${meta}"
log_ok "function metadata"

log_step "CLI invoke --fn-shasum"
err="${DATA_DIR}/invoke.err"
body="$(cli invoke "${NAME}" --url "${HOST_URL}" -d '{}' \
    --fn-shasum "${HASH}" 2>"${err}")" \
    || { dump_logs; cat "${err}" >&2 || true; die "invoke failed"; }

[[ "${body}" == "${EXPECTED_BODY}" ]] || die "body: ${body}"
grep -q "x-nitrum-fn-shasum=${HASH}" "${err}" \
    || die "missing/mismatched shasum header (expected ${HASH}): $(cat "${err}")"
while IFS= read -r line; do
    [[ -n "${line}" ]] && log_note "${line}"
done < "${err}"
log_ok "invoke after deploy (hash verified)"

log_step "unknown function → 404"
code="$(curl -sS -o /dev/null -w '%{http_code}' -X POST \
    "${HOST_URL}/invoke/does-not-exist" \
    -H 'content-type: application/json' \
    -d '{}')"
[[ "${code}" == "404" ]] || die "expected 404 for missing fn, got ${code}"
log_ok "missing function 404"

dump_logs

log_pass "local e2e passed" \
    store "Floci S3+DynamoDB" \
    api "${API_URL}" \
    host "${HOST_URL}" \
    project "${PROJECT}"
