#!/usr/bin/env bash
# Build hello-world from source, deploy, invoke.
# Assumes api + host are already running (see CONTRIBUTING.md).
#
# Usage: from repo root, `./examples/hello-world/e2e.sh`
#
#   NITRUM_FN_API_URL / NITRUM_FN_URL   default http://127.0.0.1:8080
#   NITRUM_FN_INVOKE_URL                default http://127.0.0.1:8081
#   NITRUM_FN_PCR0                      optional; passed to invoke if set
#   NITRUM_FN_BIN                       optional CLI override

set -euo pipefail

EXAMPLE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${EXAMPLE}/../.." && pwd)"
E2E_DIR="${REPO_ROOT}/tests/e2e"
# shellcheck source=../../tests/e2e/lib.sh
source "${E2E_DIR}/lib.sh"

# --- constants ---
TARGET="${TARGET:-wasm32-unknown-unknown}"
NAME="hello-world"
WASM_SRC="${EXAMPLE}/target/${TARGET}/release/hello_world.wasm"
EXPECTED_BODY='{"message":"Hello, world!"}'
API_URL="${NITRUM_FN_API_URL:-${NITRUM_FN_URL:-http://127.0.0.1:8080}}"
INVOKE_URL="${NITRUM_FN_INVOKE_URL:-http://127.0.0.1:8081}"
API_URL="${API_URL%/}"
INVOKE_URL="${INVOKE_URL%/}"

# --- required ---
require_cmd cargo
require_cmd rustup
require_cmd curl

log_banner "nitrum-fn  hello-world e2e"
log_step "parameters"
log_kv "name" "${NAME}"
log_kv "wasm" "${WASM_SRC}"
log_kv "api" "${API_URL}"
log_kv "invoke" "${INVOKE_URL}"
log_kv "cli" "$(cli_display)"

curl -sf "${API_URL}/healthz" >/dev/null \
    || die "API not reachable at ${API_URL} (start the local stack: CONTRIBUTING.md)"
invoke_health=(-sf)
[[ "${INVOKE_URL}" == https://* ]] && invoke_health=(-skf)
curl "${invoke_health[@]}" "${INVOKE_URL}/healthz" >/dev/null \
    || die "host not reachable at ${INVOKE_URL} (start the local stack: CONTRIBUTING.md)"

log_step "build ${NAME} wasm"
rustup target add "${TARGET}" >/dev/null
cargo build --manifest-path "${EXAMPLE}/Cargo.toml" --target "${TARGET}" --release
require_file "${WASM_SRC}"
log_ok "wasm built"

log_step "CLI deploy ${NAME}"
cli deploy "${WASM_SRC}" --name "${NAME}" --url "${API_URL}" \
    || die "deploy failed (${API_URL})"
log_ok "deployed"

log_step "CLI describe"
desc="$(cli describe "${WASM_SRC}")" || die "describe failed"
HASH="$(printf '%s\n' "${desc}" | awk -F= '/^hash=/{print $2}')"
[[ -n "${HASH}" ]] || die "describe missing hash=: ${desc}"
log_ok "hash=${HASH}"

log_step "CLI invoke"
err="$(mktemp)"
invoke_args=(invoke "${NAME}" --url "${INVOKE_URL}" -d '{}' --fn-shasum "${HASH}")
if [[ "${INVOKE_URL}" == https://* ]]; then
    invoke_args+=(--insecure)
fi
if [[ -n "${NITRUM_FN_PCR0:-}" ]]; then
    invoke_args+=(--pcr0 "${NITRUM_FN_PCR0}")
fi
body="$(cli "${invoke_args[@]}" 2>"${err}")" \
    || { cat "${err}" >&2; die "invoke failed"; }
[[ "${body}" == "${EXPECTED_BODY}" ]] || die "body: ${body}"
grep -q "x-nitrum-fn-shasum=${HASH}" "${err}" \
    || die "missing/mismatched shasum header (expected ${HASH}): $(cat "${err}")"
while IFS= read -r line; do
    [[ -n "${line}" ]] && log_note "${line}"
done < "${err}"
log_ok "${body}"

log_pass "hello-world e2e passed" \
    api "${API_URL}" \
    invoke "${INVOKE_URL}"
