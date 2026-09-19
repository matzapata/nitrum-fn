#!/usr/bin/env bash
# Staging oracle: deploy wasm → setEnclave → (invoke → submit → read) × 2
#
# First submit cold-caches the Nitro leaf (`CacheCerts`). The second take skips
# that and only `updatePrice`s (warm path, newer NSM timestamp).
#
# Environment (`set -a && source .env && set +a` — see .env.example):
#   PRIVATE_KEY, BASE_SEPOLIA_RPC_URL, ORACLE_ADDRESS
#   optional NITRUM_FN_API_URL / NITRUM_FN_INVOKE_URL / NITRUM_FN_PCR0
#     (or API_URL / INVOKE_URL / PCR0). Stale URLs NXDOMAIN-fallback to terraform.
#   NITRUM_ORACLE_SKIP_CACHE_CERTS   set to 1 to skip CacheCerts (leaf already cached)
#   NITRUM_ORACLE_GAS_LIMIT          default 16000000
#
# Requires a deployed staging stack (`enable_enclave = true`) and a deployed
# NitrumOracle. Does not apply Terraform or run Deploy.s.sol.
#
# Usage: from repo root, `./examples/oracle/e2e.sh`

set -euo pipefail

ORACLE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${ORACLE_DIR}/../.." && pwd)"
E2E_DIR="${REPO_ROOT}/tests/e2e"

# shellcheck source=../../tests/e2e/lib.sh
source "${E2E_DIR}/lib.sh"

# --- constants ---
CONTRACTS="${ORACLE_DIR}/contracts"
ENCLAVE="${ORACLE_DIR}/enclave"
CONFIG="${ENCLAVE}/nitrum.toml"
HINTS_JS="${CONTRACTS}/lib/nitro-validator/tools/p384_hints.js"
TARGET="${TARGET:-wasm32-unknown-unknown}"
NAME="oracle"
WASM_SRC="${ENCLAVE}/target/${TARGET}/release/oracle.wasm"
INVOKE_BODY='{"ids":["eth"]}'
READ_ID="eth"
TF_DIR="${NITRUM_FN_E2E_TF_DIR:-${REPO_ROOT}/infra/envs/staging}"
TFVARS="${TF_DIR}/terraform.tfvars"
API_TIMEOUT="${NITRUM_FN_E2E_API_TIMEOUT:-90}"
INVOKE_TIMEOUT="${NITRUM_FN_E2E_INVOKE_TIMEOUT:-600}"
POLL_SECONDS="${NITRUM_FN_E2E_POLL_SECONDS:-5}"
ENCLAVE_TLS_INSECURE="${ENCLAVE_TLS_INSECURE:-1}"

# --- required ---
require_cmd curl
require_cmd cargo
require_cmd rustup
require_cmd forge
require_cmd cast
require_cmd python3
require_cmd node
require_dir "${ORACLE_DIR}"
require_dir "${ENCLAVE}"
require_dir "${CONTRACTS}"
require_file "${CONFIG}"
[[ -f "${HINTS_JS}" ]] \
    || die "missing ${HINTS_JS} (git submodule update --init --recursive)"

# Shell may use API_URL / INVOKE_URL / PCR0; harness prefers NITRUM_FN_*.
if [[ -z "${NITRUM_FN_API_URL:-}" && -n "${API_URL:-}" ]]; then
    NITRUM_FN_API_URL="${API_URL}"
fi
if [[ -z "${NITRUM_FN_INVOKE_URL:-}" && -n "${INVOKE_URL:-}" ]]; then
    NITRUM_FN_INVOKE_URL="${INVOKE_URL}"
fi
if [[ -z "${NITRUM_FN_PCR0:-}" && -n "${PCR0:-}" ]]; then
    NITRUM_FN_PCR0="${PCR0}"
fi

GAS_LIMIT="${NITRUM_ORACLE_GAS_LIMIT:-16000000}"
CACHE_CERTS=1
if [[ "${NITRUM_ORACLE_SKIP_CACHE_CERTS:-0}" == "1" ]]; then
    CACHE_CERTS=0
fi

require_var PRIVATE_KEY
require_var BASE_SEPOLIA_RPC_URL
require_var ORACLE_ADDRESS
[[ "${PRIVATE_KEY}" != "0x" ]] \
    || die "missing PRIVATE_KEY (placeholder 0x — source .env; see .env.example)"
[[ "${ORACLE_ADDRESS}" != "0x" ]] \
    || die "missing ORACLE_ADDRESS (placeholder 0x — source .env; deploy NitrumOracle first)"

log_banner "nitrum-fn  oracle e2e"
log_step "parameters"
log_kv "name" "${NAME}"
log_kv "wasm" "${WASM_SRC}"
log_kv "oracle" "${ORACLE_ADDRESS}"
log_kv "rpc" "${BASE_SEPOLIA_RPC_URL}"
log_kv "gas" "${GAS_LIMIT}"
log_kv "cache" "${CACHE_CERTS}"
log_kv "tf" "${TF_DIR}"
log_kv "tls" "insecure=${ENCLAVE_TLS_INSECURE}"
log_kv "cli" "$(cli_display)"

cloud_resolve
log_step "endpoints"
log_kv "api" "${API_URL}"
log_kv "invoke" "${INVOKE_URL}"
log_kv "pcr0" "${PCR0}"

# PCR0 as 96 hex chars (48 bytes), no 0x — CLI pin.
pcr0_hex() {
    local h="${1#0x}"
    h="${h#0X}"
    printf '%s' "${h}" | tr 'A-F' 'a-f'
}

bytes_to_hex() {
    python3 -c 'import sys; print("0x" + sys.stdin.buffer.read().hex())'
}

step_build_oracle() {
    log_step "build ${NAME} wasm"
    rustup target add "${TARGET}" >/dev/null
    cargo build --manifest-path "${ENCLAVE}/Cargo.toml" --target "${TARGET}" --release
    [[ -f "${WASM_SRC}" ]] || die "wasm missing at ${WASM_SRC}"
    log_ok "wasm built"
}

step_describe() {
    log_step "CLI describe"
    local desc
    desc="$(cli describe "${WASM_SRC}")" || die "CLI describe failed"
    HASH="$(printf '%s\n' "${desc}" | awk -F= '/^hash=/{print $2}')"
    [[ -n "${HASH}" ]] || die "describe missing hash=: ${desc}"
    CONTENT_HASH="0x${HASH}"
    export CONTENT_HASH
    log_ok "content hash ${CONTENT_HASH}"
}

step_deploy_oracle() {
    log_step "CLI deploy ${NAME}"
    cli deploy "${WASM_SRC}" --name "${NAME}" --url "${API_URL}" --config "${CONFIG}" \
        || die "deploy failed (ALB=${API_URL})"

    log_step "GET /functions/${NAME}"
    local meta
    meta="$(curl -sf "${API_URL}/functions/${NAME}")" \
        || die "GET function metadata failed"
    echo "${meta}" | grep -q "\"name\":\"${NAME}\"" || die "metadata name: ${meta}"
    log_ok "catalog metadata ok"
}

step_set_enclave() {
    local raw
    raw="$(pcr0_hex "${PCR0}")"
    [[ "${#raw}" -eq 96 ]] || die "PCR0 must be 48 bytes (96 hex chars), got ${#raw}"
    export PCR0="0x${raw}"

    log_step "setEnclave"
    log_kv "oracle" "${ORACLE_ADDRESS}"
    log_kv "pcr0" "${PCR0}"
    (
        cd "${CONTRACTS}"
        forge script script/SetEnclave.s.sol:SetEnclave \
            --rpc-url "${BASE_SEPOLIA_RPC_URL}" \
            --broadcast
    ) || die "setEnclave failed"
    log_ok "enclave pinned"
}

# $1 = round label, $2 = 1 to CacheCerts first
round_invoke_submit_read() {
    local round="$1"
    local cache_certs="$2"
    local work att err body

    work="$(mktemp -d)"
    att="${work}/attestation.bin"
    err="${work}/invoke.err"

    log_step "[${round}] CLI invoke (attested)"
    body="$(
        cli invoke "${NAME}" --url "${INVOKE_URL}" --insecure \
            -d "${INVOKE_BODY}" \
            --fn-shasum "${HASH}" \
            --pcr0 "$(pcr0_hex "${PCR0}")" \
            --attestation-out "${att}" 2>"${err}"
    )" || { cat "${err}" >&2; die "invoke failed"; }
    [[ -s "${att}" ]] || die "empty attestation at ${att}"
    [[ "${body}" == *'"prices":['* ]] || die "unexpected invoke body: ${body}"
    [[ "${body}" != *'"error"'* ]] || die "invoke error body: ${body}"
    log_ok "invoke body"
    log_note "${body}"

    # CLI println adds a newline; command substitution strips it. Hash the exact JSON.
    export ATTESTATION_HEX
    export BODY_HEX
    ATTESTATION_HEX="$(bytes_to_hex < "${att}")"
    BODY_HEX="$(printf '%s' "${body}" | bytes_to_hex)"

    (
        cd "${CONTRACTS}"
        if [[ "${cache_certs}" == "1" ]]; then
            log_step "[${round}] CacheCerts calldata"
            log_note "P-384 hints via node; no local verify (Foundry MODEXP)"
            rm -f cacheCerts.to cacheCerts.[0-9].data cacheCerts.[0-9][0-9].data
            forge script script/UpdatePrices.s.sol:CacheCerts --ffi \
                --rpc-url "${BASE_SEPOLIA_RPC_URL}" \
                || die "CacheCerts script failed"
            if [[ -s cacheCerts.0.data ]]; then
                to="$(cat cacheCerts.to)"
                [[ -n "${to}" ]] || die "missing cacheCerts.to"
                i=0
                while [[ -s "cacheCerts.${i}.data" ]]; do
                    log_step "[${round}] cast send cacheCerts.${i}"
                    cast send "${to}" "$(cat "cacheCerts.${i}.data")" \
                        --rpc-url "${BASE_SEPOLIA_RPC_URL}" \
                        --private-key "${PRIVATE_KEY}" \
                        --gas-limit "${GAS_LIMIT}" \
                        || die "cacheCerts.${i} send failed"
                    i=$((i + 1))
                done
            else
                log_ok "certs already cached"
            fi
        else
            log_step "[${round}] skip CacheCerts (warm path)"
        fi

        log_step "[${round}] UpdatePrices calldata"
        forge script script/UpdatePrices.s.sol:UpdatePrices --ffi \
            --rpc-url "${BASE_SEPOLIA_RPC_URL}" \
            || die "UpdatePrices script failed"
        [[ -s updatePrice.data ]] || die "missing updatePrice.data"
    )

    log_step "[${round}] cast send updatePrice"
    cast send "${ORACLE_ADDRESS}" "$(cat "${CONTRACTS}/updatePrice.data")" \
        --rpc-url "${BASE_SEPOLIA_RPC_URL}" \
        --private-key "${PRIVATE_KEY}" \
        --gas-limit "${GAS_LIMIT}" \
        || die "updatePrice send failed"

    log_step "[${round}] getPrice(${READ_ID})"
    local price data
    price="$(cast call "${ORACLE_ADDRESS}" "getPrice(string)(uint256)" "${READ_ID}" \
        --rpc-url "${BASE_SEPOLIA_RPC_URL}")" \
        || die "getPrice failed"
    data="$(cast call "${ORACLE_ADDRESS}" "getPriceData(string)(uint256,uint64)" "${READ_ID}" \
        --rpc-url "${BASE_SEPOLIA_RPC_URL}")" \
        || die "getPriceData failed"
    log_kv "getPrice" "${price}"
    log_kv "data" "${data}"
    [[ "${price}" != "0" ]] || die "getPrice returned 0"
    log_ok "price on-chain"

    rm -rf "${work}"
}

step_wait_api
step_build_oracle
step_describe
step_deploy_oracle
step_wait_enclave
step_set_enclave
round_invoke_submit_read 1 "${CACHE_CERTS}"
round_invoke_submit_read 2 0

log_pass "oracle e2e passed" \
    api "${API_URL}" \
    invoke "${INVOKE_URL}" \
    pcr0 "${PCR0}" \
    oracle "${ORACLE_ADDRESS}"
