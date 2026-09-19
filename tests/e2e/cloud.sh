#!/usr/bin/env bash
# Staging e2e: nitrum-fn new → build → deploy → wait enclave → hash/PCR0 tests.
#
# Environment (optional; `set -a && source .env && set +a` — see .env.example):
#   NITRUM_FN_BIN            optional override command; default cargo run -q --bin nitrum-fn
#   NITRUM_FN_E2E_RUST_LOG   tracing filter for default cargo path (default: warn,cli=info)
#   NITRUM_FN_E2E_PARENT_DIR / NITRUM_FN_E2E_INIT_NAME   scaffold workspace (defaults under target/)
#   NITRUM_FN_API_URL        optional; else terraform output api_url
#   NITRUM_FN_INVOKE_URL     optional; else terraform output invoke_url
#   NITRUM_FN_PCR0           optional; else terraform output pcr0
#   NITRUM_FN_E2E_TF_DIR     default: infra/envs/staging
#   ENCLAVE_TLS_INSECURE     default 1
#   NITRUM_FN_E2E_API_TIMEOUT / NITRUM_FN_E2E_INVOKE_TIMEOUT   default 90 / 600
#   NITRUM_FN_E2E_POLL_SECONDS                                default 5
#
# Requires a deployed staging stack with enable_enclave = true.
# This script does not apply or destroy Terraform.
#
# Usage: from repo root, `./tests/e2e/cloud.sh`

set -euo pipefail

E2E_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${E2E_DIR}/../.." && pwd)"

# shellcheck source=lib.sh
source "${E2E_DIR}/lib.sh"

# --- constants ---
EXPECTED_BODY='{"message":"Hello, world!"}'
TF_DIR="${NITRUM_FN_E2E_TF_DIR:-${REPO_ROOT}/infra/envs/staging}"
TFVARS="${TF_DIR}/terraform.tfvars"
API_TIMEOUT="${NITRUM_FN_E2E_API_TIMEOUT:-90}"
INVOKE_TIMEOUT="${NITRUM_FN_E2E_INVOKE_TIMEOUT:-600}"
POLL_SECONDS="${NITRUM_FN_E2E_POLL_SECONDS:-5}"
ENCLAVE_TLS_INSECURE="${ENCLAVE_TLS_INSECURE:-1}"

e2e_init_workspace

# --- required ---
require_cmd curl
require_cmd cargo
require_cmd rustup

log_banner "nitrum-fn  cloud e2e"
log_step "parameters"
log_kv "name" "${NAME}"
log_kv "parent" "${PARENT}"
log_kv "tf" "${TF_DIR}"
log_kv "tls" "insecure=${ENCLAVE_TLS_INSECURE}"
log_kv "api_wait" "${API_TIMEOUT}s"
log_kv "inv_wait" "${INVOKE_TIMEOUT}s"
log_kv "cli" "$(cli_display)"

cloud_resolve
log_step "endpoints"
log_kv "api" "${API_URL}"
log_kv "invoke" "${INVOKE_URL}"
log_kv "pcr0" "${PCR0}"

step_wait_api
step_scaffold_and_build

log_step "CLI deploy ${NAME}"
cli deploy "${WASM_SRC}" --name "${NAME}" --url "${API_URL}" \
    || die "deploy failed (ALB=${API_URL})"

log_step "GET /functions/${NAME}"
meta="$(curl -sf "${API_URL}/functions/${NAME}")" \
    || die "GET function metadata failed"
echo "${meta}" | grep -q "\"name\":\"${NAME}\"" || die "metadata name: ${meta}"
log_ok "catalog metadata ok"

step_wait_enclave

invoke_curl_args

log_step "curl invoke after deploy"
headers="$(mktemp)"
body="$(curl "${CURL_ARGS[@]}" -D "${headers}" -X POST \
    "${INVOKE_URL}/invoke/${NAME}" \
    -H 'content-type: application/json' \
    -d '{}')"
grep -qi '^HTTP/.* 200' "${headers}" || die "status not 200 ($(head -1 "${headers}")); body=${body}"
[[ "${body}" == "${EXPECTED_BODY}" ]] || die "body: ${body}"
log_ok "200  ${body}"

log_step "unknown function → 404"
code="$(curl "${CURL_ARGS[@]}" -o /dev/null -w '%{http_code}' -X POST \
    "${INVOKE_URL}/invoke/does-not-exist" \
    -H 'content-type: application/json' \
    -d '{}')"
[[ "${code}" == "404" ]] || die "expected 404 for missing fn, got ${code}"
log_ok "404"

log_step "CLI invoke --fn-shasum"
err="$(mktemp)"
body="$(
    unset NITRUM_FN_PCR0
    cli invoke "${NAME}" --url "${INVOKE_URL}" --insecure -d '{}' \
        --fn-shasum "${HASH}" 2>"${err}"
)" \
    || { cat "${err}" >&2; die "CLI invoke --fn-shasum failed"; }
[[ "${body}" == "${EXPECTED_BODY}" ]] || die "CLI --fn-shasum body: ${body}"
grep -q 'x-nitrum-fn-shasum=' "${err}" || die "missing x-nitrum-fn-shasum on stderr: $(cat "${err}")"
if grep -q 'attestation: ok' "${err}"; then
    die "unexpected attestation verify without --pcr0: $(cat "${err}")"
fi
while IFS= read -r line; do
    [[ -n "${line}" ]] && log_note "${line}"
done < "${err}"
log_ok "fn-shasum verified"

log_step "CLI invoke --fn-shasum --pcr0 (NSM attestation)"
att="$(mktemp)"
body="$(cli invoke "${NAME}" --url "${INVOKE_URL}" --insecure -d '{}' \
    --fn-shasum "${HASH}" --pcr0 "${PCR0}" --attestation-out "${att}" 2>"${err}")" \
    || { cat "${err}" >&2; die "CLI invoke --pcr0 failed"; }
[[ "${body}" == "${EXPECTED_BODY}" ]] || die "CLI --pcr0 body: ${body}"
[[ -s "${att}" ]] || die "empty --attestation-out file"
if grep -qE 'x-nitrum-fn-shasum=|attestation: ok|document=|written=' "${err}"; then
    die "expected silent stderr with --attestation-out: $(cat "${err}")"
fi
att_bytes="$(wc -c < "${att}" | tr -d ' ')"
log_ok "NSM attestation written (${att_bytes} bytes)"

log_step "negative: wrong --fn-shasum"
bad_hash="$(printf '0%.0s' {1..64})"
if cli invoke "${NAME}" --url "${INVOKE_URL}" --insecure -d '{}' \
    --fn-shasum "${bad_hash}" --pcr0 "${PCR0}" >/dev/null 2>"${err}"; then
    die "expected hash mismatch to fail"
fi
grep -qi 'hash mismatch' "${err}" || die "expected hash mismatch error: $(cat "${err}")"
log_ok "rejected (hash mismatch)"

log_step "negative: wrong --pcr0"
bad_pcr0="$(printf '0%.0s' {1..96})"
if cli invoke "${NAME}" --url "${INVOKE_URL}" --insecure -d '{}' \
    --fn-shasum "${HASH}" --pcr0 "${bad_pcr0}" >/dev/null 2>"${err}"; then
    die "expected PCR0 mismatch to fail"
fi
grep -qiE 'attestation verify failed|PCR' "${err}" \
    || die "expected PCR/attestation failure: $(cat "${err}")"
log_ok "rejected (PCR/attestation)"

log_pass "cloud e2e passed" \
    api "${API_URL}" \
    invoke "${INVOKE_URL}" \
    project "${PROJECT}"
