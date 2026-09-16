#!/usr/bin/env bash
# Linear workflow:
#   resolve staging URLs → wait API → build wasm → CLI deploy → wait enclave → tests
#
# Environment (see tests/e2e/.env.example):
#   NITRUM_FN_BIN            optional override command; default:
#                            cargo run -q ... (quiet compile; see NITRUM_FN_E2E_RUST_LOG)
#   NITRUM_FN_E2E_RUST_LOG   tracing filter for default cargo path (default: warn,cli=info)
#   NITRUM_FN_API_URL        optional; else terraform output api_url
#   NITRUM_FN_INVOKE_URL     optional; else terraform output invoke_url
#                            Env URLs that NXDOMAIN (destroyed ALB/NLB) fall back to terraform.
#   NITRUM_FN_PCR0           optional; else eif_image_sha384 in terraform.tfvars
#   NITRUM_FN_E2E_TF_DIR     default: infra/envs/staging
#   NITRUM_FN_E2E_NAME       default: hello-world
#   ENCLAVE_TLS_INSECURE     default 1 (self-signed / hostname mismatch on NLB DNS)
#   NITRUM_FN_E2E_API_TIMEOUT / NITRUM_FN_E2E_INVOKE_TIMEOUT   default 90 / 600
#   NITRUM_FN_E2E_POLL_SECONDS                                default 5
#
# Requires a deployed staging stack with enable_enclave = true.
# This script does not apply or destroy Terraform.
#
# Usage: from repo root, `./tests/e2e/cloud.sh`

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

if [[ -f "${SCRIPT_DIR}/.env" ]]; then
    set -a
    # shellcheck source=/dev/null
    source "${SCRIPT_DIR}/.env"
    set +a
fi

TF_DIR="${NITRUM_FN_E2E_TF_DIR:-${REPO_ROOT}/infra/envs/staging}"
TFVARS="${TF_DIR}/terraform.tfvars"
EXAMPLE="${REPO_ROOT}/examples/hello-world"
TARGET="wasm32-unknown-unknown"
WASM_SRC="${EXAMPLE}/target/${TARGET}/release/hello_world.wasm"
NAME="${NITRUM_FN_E2E_NAME:-hello-world}"
API_TIMEOUT="${NITRUM_FN_E2E_API_TIMEOUT:-90}"
INVOKE_TIMEOUT="${NITRUM_FN_E2E_INVOKE_TIMEOUT:-600}"
POLL_SECONDS="${NITRUM_FN_E2E_POLL_SECONDS:-5}"
ENCLAVE_TLS_INSECURE="${ENCLAVE_TLS_INSECURE:-1}"
EXPECTED_BODY='{"message":"Hello, world!"}'

API_URL=""
INVOKE_URL=""
PCR0=""

die() {
    echo "error: $*" >&2
    exit 1
}

cli() {
    local -a cmd
    if [[ -n "${NITRUM_FN_BIN:-}" ]]; then
        # shellcheck disable=SC2206
        cmd=(${NITRUM_FN_BIN})
        "${cmd[@]}" -- "$@"
    else
        RUST_LOG="${NITRUM_FN_E2E_RUST_LOG:-warn,cli=info}" \
            cargo run -q --manifest-path "${REPO_ROOT}/Cargo.toml" --bin nitrum-fn -- "$@"
    fi
}

tf_output() {
    terraform -chdir="${TF_DIR}" output -raw "$1"
}

pcr0_from_tfvars() {
    [[ -f "${TFVARS}" ]] || return 0
    sed -nE 's/^[[:space:]]*eif_image_sha384[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/p' "${TFVARS}" | head -1
}

invoke_curl_args() {
    CURL_ARGS=(-sS --connect-timeout 5 --max-time 60)
    if [[ "${ENCLAVE_TLS_INSECURE}" == "1" ]]; then
        CURL_ARGS+=(-k)
    fi
}

url_host() {
    local u="${1#*://}"
    u="${u%%/*}"
    u="${u%%:*}"
    printf '%s' "$u"
}

# True if hostname has DNS records. NXDOMAIN is not "not ready yet".
dns_resolves() {
    local host="$1"
    if command -v python3 >/dev/null 2>&1; then
        python3 -c 'import socket, sys; socket.getaddrinfo(sys.argv[1], None)' "$host" >/dev/null 2>&1
        return
    fi
    local rc=0
    curl -sS -o /dev/null --connect-timeout 2 --max-time 3 "http://${host}/" || rc=$?
    [[ "${rc}" -ne 6 ]]
}

# stdout: URL to use. If an env override NXDOMAINs, fall back to terraform.
resolve_endpoint() {
    local url="$1"
    local from_env="$2"
    local tf_key="$3"
    local label="$4"
    local source host

    if [[ "${from_env}" == "1" ]]; then
        source="env"
    else
        source="terraform"
    fi
    echo "=== ${label} from ${source}: ${url} ===" >&2

    host="$(url_host "${url}")"
    if dns_resolves "${host}"; then
        printf '%s' "${url}"
        return 0
    fi

    if [[ "${from_env}" == "1" ]]; then
        echo "warning: ${label} URL from env does not resolve (${host}); falling back to terraform output ${tf_key}" >&2
        command -v terraform >/dev/null 2>&1 \
            || die "${label} hostname does not resolve: ${host} (install terraform or update NITRUM_FN_* URLs)"
        echo "=== resolve terraform output ${tf_key} (${TF_DIR}) ===" >&2
        url="$(tf_output "${tf_key}")" || die "terraform output ${tf_key} failed"
        url="${url%/}"
        echo "=== ${label} from terraform: ${url} ===" >&2
        host="$(url_host "${url}")"
        if dns_resolves "${host}"; then
            printf '%s' "${url}"
            return 0
        fi
    fi

    die "${label} hostname does not resolve: ${host} (${url})"
}

wait_http() {
    local url="$1"
    local timeout_seconds="$2"
    local label="$3"

    command -v curl >/dev/null 2>&1 || die "curl not found (needed to wait for ${label})"

    echo "=== wait ${label} (timeout ${timeout_seconds}s): ${url} ==="

    local deadline now remaining sleep_for rc
    deadline=$((SECONDS + timeout_seconds))
    local -a curl_args
    # Do not treat TCP success as ready: ALB 503 (no healthy targets) is still
    # curl exit 0. Require HTTP 200, matching the target-group health check.
    curl_args=(-sS -o /dev/null -w '%{http_code}' --connect-timeout 5 --max-time 10)

    while true; do
        local rc=0 code="000"
        code="$(curl "${curl_args[@]}" "${url}")" || rc=$?
        if ((rc == 0)) && [[ "${code}" == "200" ]]; then
            echo "=== ${label} ready ==="
            return 0
        fi
        if ((rc == 6)); then
            die "${label} hostname does not resolve (${url}). Unset stale NITRUM_FN_API_URL / NITRUM_FN_INVOKE_URL to use terraform outputs"
        fi

        now=${SECONDS}
        if ((now >= deadline)); then
            die "${label} did not become ready within ${timeout_seconds}s (${url})"
        fi

        remaining=$((deadline - now))
        echo "waiting for ${label}... http=${code} rc=${rc} (${remaining}s remaining)"
        sleep_for="${POLL_SECONDS}"
        if ((remaining < POLL_SECONDS)); then
            sleep_for="${remaining}"
        fi
        sleep "${sleep_for}"
    done
}

step_resolve() {
    local api_from_env=0 invoke_from_env=0
    API_URL="${NITRUM_FN_API_URL:-}"
    INVOKE_URL="${NITRUM_FN_INVOKE_URL:-}"
    PCR0="${NITRUM_FN_PCR0:-}"

    [[ -n "${API_URL}" ]] && api_from_env=1
    [[ -n "${INVOKE_URL}" ]] && invoke_from_env=1

    if [[ -z "${API_URL}" || -z "${INVOKE_URL}" ]]; then
        command -v terraform >/dev/null 2>&1 || die "set NITRUM_FN_API_URL / NITRUM_FN_INVOKE_URL or install terraform"
        echo "=== resolve terraform outputs (${TF_DIR}) ==="
        if [[ -z "${API_URL}" ]]; then
            API_URL="$(tf_output api_url)" || die "terraform output api_url failed"
        fi
        if [[ -z "${INVOKE_URL}" ]]; then
            INVOKE_URL="$(tf_output invoke_url)" || die "terraform output invoke_url failed"
        fi
    fi

    if [[ -z "${PCR0}" ]]; then
        PCR0="$(pcr0_from_tfvars)"
    fi

    API_URL="${API_URL%/}"
    INVOKE_URL="${INVOKE_URL%/}"

    if [[ -z "${API_URL}" || "${API_URL}" == "null" ]]; then
        die "missing API URL (NITRUM_FN_API_URL or terraform output api_url)"
    fi
    if [[ -z "${INVOKE_URL}" || "${INVOKE_URL}" == "null" ]]; then
        die "missing invoke URL (NITRUM_FN_INVOKE_URL or terraform output invoke_url; enable_enclave must be true)"
    fi
    if [[ -z "${PCR0}" ]]; then
        die "set NITRUM_FN_PCR0 or eif_image_sha384 in ${TFVARS}"
    fi

    API_URL="$(resolve_endpoint "${API_URL}" "${api_from_env}" api_url api)"
    INVOKE_URL="$(resolve_endpoint "${INVOKE_URL}" "${invoke_from_env}" invoke_url invoke)"
}

step_wait_api() {
    wait_http "${API_URL}/healthz" "${API_TIMEOUT}" "API"
}

step_build() {
    echo "=== build ${NAME} wasm ==="
    rustup target add "${TARGET}" >/dev/null
    cargo build --manifest-path "${EXAMPLE}/Cargo.toml" --target "${TARGET}" --release
    [[ -f "${WASM_SRC}" ]] || die "wasm missing at ${WASM_SRC}"
}

step_deploy() {
    echo "=== CLI deploy ${NAME} ==="
    cli deploy "${WASM_SRC}" --name "${NAME}" --url "${API_URL}" \
        || die "deploy failed (ALB=${API_URL})"

    echo "=== GET /functions/${NAME} ==="
    local meta
    meta="$(curl -sf "${API_URL}/functions/${NAME}")" \
        || die "GET function metadata failed"
    echo "${meta}" | grep -q "\"name\":\"${NAME}\"" || die "metadata name: ${meta}"
}

step_wait_enclave() {
    command -v curl >/dev/null 2>&1 || die "curl not found (needed to wait for enclave)"

    local status_url="${INVOKE_URL}/.well-known/enclave/status"
    local health_url="${INVOKE_URL}/healthz"

    echo "=== wait enclave (timeout ${INVOKE_TIMEOUT}s): ${INVOKE_URL} ==="

    local deadline now remaining sleep_for rc_status rc_health code_status code_health
    deadline=$((SECONDS + INVOKE_TIMEOUT))
    local -a curl_args
    curl_args=(-sS -o /dev/null -w '%{http_code}' --connect-timeout 5 --max-time 10)
    if [[ "${ENCLAVE_TLS_INSECURE}" == "1" ]]; then
        curl_args+=(-k)
    fi

    while true; do
        rc_status=0
        rc_health=0
        code_status="000"
        code_health="000"
        code_status="$(curl "${curl_args[@]}" "${status_url}")" || rc_status=$?
        if ((rc_status == 0)) && [[ "${code_status}" == "200" ]]; then
            echo "=== enclave is responsive ==="
            return 0
        fi
        code_health="$(curl "${curl_args[@]}" "${health_url}")" || rc_health=$?
        if ((rc_health == 0)) && [[ "${code_health}" == "200" ]]; then
            echo "=== enclave is responsive ==="
            return 0
        fi
        if ((rc_status == 6 && rc_health == 6)); then
            die "enclave hostname does not resolve (${INVOKE_URL}). Unset stale NITRUM_FN_INVOKE_URL to use terraform outputs"
        fi

        now=${SECONDS}
        if ((now >= deadline)); then
            die "enclave did not become ready within ${INVOKE_TIMEOUT}s (${INVOKE_URL}). Check ASG / control-plane logs"
        fi

        remaining=$((deadline - now))
        echo "waiting for enclave... (${remaining}s remaining)"
        sleep_for="${POLL_SECONDS}"
        if ((remaining < POLL_SECONDS)); then
            sleep_for="${remaining}"
        fi
        sleep "${sleep_for}"
    done
}

step_tests() {
    local headers body code err bad_hash bad_pcr0 desc hash

    invoke_curl_args

    echo "=== curl invoke after deploy ==="
    headers="$(mktemp)"
    body="$(curl "${CURL_ARGS[@]}" -D "${headers}" -X POST \
        "${INVOKE_URL}/invoke/${NAME}" \
        -H 'content-type: application/json' \
        -d '{}')"
    grep -qi '^HTTP/.* 200' "${headers}" || die "status not 200 ($(head -1 "${headers}")); body=${body}"
    [[ "${body}" == "${EXPECTED_BODY}" ]] || die "body: ${body}"

    echo "=== unknown function → 404 ==="
    code="$(curl "${CURL_ARGS[@]}" -o /dev/null -w '%{http_code}' -X POST \
        "${INVOKE_URL}/invoke/does-not-exist" \
        -H 'content-type: application/json' \
        -d '{}')"
    [[ "${code}" == "404" ]] || die "expected 404 for missing fn, got ${code}"

    echo "=== CLI describe ==="
    desc="$(cli describe "${WASM_SRC}")" || die "CLI describe failed"
    hash="$(printf '%s\n' "${desc}" | awk -F= '/^hash=/{print $2}')"
    [[ -n "${hash}" ]] || die "describe missing hash=: ${desc}"

    echo "=== CLI invoke --fn-shasum ==="
    err="$(mktemp)"
    body="$(cli invoke "${NAME}" --url "${INVOKE_URL}" --insecure -d '{}' \
        --fn-shasum "${hash}" 2>"${err}")" \
        || { cat "${err}" >&2; die "CLI invoke --fn-shasum failed"; }
    [[ "${body}" == "${EXPECTED_BODY}" ]] || die "CLI --fn-shasum body: ${body}"
    grep -q 'x-nitrum-fn-hash=' "${err}" || die "missing x-nitrum-fn-hash on stderr: $(cat "${err}")"
    if grep -q 'attestation: ok' "${err}"; then
        die "unexpected attestation verify without --pcr0: $(cat "${err}")"
    fi
    cat "${err}" >&2

    echo "=== CLI invoke --fn-shasum --pcr0 (NSM attestation) ==="
    att="$(mktemp)"
    body="$(cli invoke "${NAME}" --url "${INVOKE_URL}" --insecure -d '{}' \
        --fn-shasum "${hash}" --pcr0 "${PCR0}" --attestation-out "${att}" 2>"${err}")" \
        || { cat "${err}" >&2; die "CLI invoke --pcr0 failed"; }
    [[ "${body}" == "${EXPECTED_BODY}" ]] || die "CLI --pcr0 body: ${body}"
    [[ -s "${att}" ]] || die "empty --attestation-out file"
    if grep -qE 'x-nitrum-fn-hash=|attestation: ok|document=|written=' "${err}"; then
        die "expected silent stderr with --attestation-out: $(cat "${err}")"
    fi
    echo "--- NSM attestation written ---" >&2
    wc -c "${att}" >&2
    echo "-------------------------------" >&2

    echo "=== negative: wrong --fn-shasum ==="
    bad_hash="$(printf '0%.0s' {1..64})"
    if cli invoke "${NAME}" --url "${INVOKE_URL}" --insecure -d '{}' \
        --fn-shasum "${bad_hash}" --pcr0 "${PCR0}" >/dev/null 2>"${err}"; then
        die "expected hash mismatch to fail"
    fi
    grep -qi 'hash mismatch' "${err}" || die "expected hash mismatch error: $(cat "${err}")"

    echo "=== negative: wrong --pcr0 ==="
    bad_pcr0="$(printf '0%.0s' {1..96})"
    if cli invoke "${NAME}" --url "${INVOKE_URL}" --insecure -d '{}' \
        --fn-shasum "${hash}" --pcr0 "${bad_pcr0}" >/dev/null 2>"${err}"; then
        die "expected PCR0 mismatch to fail"
    fi
    grep -qiE 'attestation verify failed|PCR' "${err}" \
        || die "expected PCR/attestation failure: $(cat "${err}")"
}

main() {
    step_resolve
    step_wait_api
    step_build
    step_deploy
    step_wait_enclave
    step_tests

    echo
    echo "=== cloud e2e passed (api=${API_URL}, invoke=${INVOKE_URL}) ==="
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
    main "$@"
fi
