#!/usr/bin/env bash
# Shared e2e helpers: logging, CLI wrapper, scaffold/build, cloud resolve/wait.
# Source from local.sh / cloud.sh / examples/*/e2e.sh — do not execute.
# Color + unicode when stderr is a TTY and NO_COLOR is unset.

_LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
E2E_DIR="${E2E_DIR:-${_LIB_DIR}}"
REPO_ROOT="${REPO_ROOT:-$(cd "${E2E_DIR}/../.." && pwd)}"

# --- logging ---

if [[ -t 2 && -z "${NO_COLOR:-}" ]]; then
    _c_reset=$'\033[0m'
    _c_dim=$'\033[2m'
    _c_bold=$'\033[1m'
    _c_cyan=$'\033[36m'
    _c_green=$'\033[32m'
    _c_yellow=$'\033[33m'
    _c_red=$'\033[31m'
    _m_step='→'
    _m_ok='✓'
    _m_fail='✗'
    _m_wait='…'
    _m_warn='!'
    _rule='────────────────────────────────────────'
else
    _c_reset='' _c_dim='' _c_bold=''
    _c_cyan='' _c_green='' _c_yellow='' _c_red=''
    _m_step='>'
    _m_ok='+'
    _m_fail='x'
    _m_wait='...'
    _m_warn='!'
    _rule='----------------------------------------'
fi

# All helpers write stderr so command substitution stays clean.
log_banner() {
    printf '\n  %b%s%b\n  %b%s%b\n' \
        "${_c_bold}" "$1" "${_c_reset}" \
        "${_c_dim}" "${_rule}" "${_c_reset}" >&2
}

log_step() {
    printf '\n  %b%s%b  %s\n' "${_c_cyan}${_c_bold}" "${_m_step}" "${_c_reset}" "$*" >&2
}

log_kv() {
    printf '      %b%-10s%b %s\n' "${_c_dim}" "$1" "${_c_reset}" "$2" >&2
}

log_note() {
    printf '      %s\n' "$*" >&2
}

log_ok() {
    printf '  %b%s%b  %s\n' "${_c_green}${_c_bold}" "${_m_ok}" "${_c_reset}" "$*" >&2
}

log_wait() {
    printf '      %b%s%b %s\n' "${_c_dim}${_c_yellow}" "${_m_wait}" "${_c_reset}${_c_dim}" "$*${_c_reset}" >&2
}

log_warn() {
    printf '  %b%s%b  %s\n' "${_c_yellow}${_c_bold}" "${_m_warn}" "${_c_reset}" "$*" >&2
}

log_pass() {
    printf '\n  %b%s%b\n  %b%s  %s%b\n' \
        "${_c_dim}" "${_rule}" "${_c_reset}" \
        "${_c_green}${_c_bold}" "${_m_ok}" "$1" "${_c_reset}" >&2
    shift
    while [[ $# -ge 2 ]]; do
        log_kv "$1" "$2"
        shift 2
    done
    printf '\n' >&2
}

die() {
    printf '  %b%s%b  %s\n' "${_c_red}${_c_bold}" "${_m_fail}" "${_c_reset}" "$*" >&2
    exit 1
}

require_var() {
    local name="$1"
    local val="${!name:-}"
    if [[ -z "${val}" || "${val}" == "null" ]]; then
        die "missing ${name}"
    fi
}

require_cmd() {
    command -v "$1" >/dev/null 2>&1 || die "missing command: $1"
}

require_file() {
    [[ -f "$1" ]] || die "missing file: $1"
}

require_dir() {
    [[ -d "$1" ]] || die "missing directory: $1"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
    die "source this file from local.sh, cloud.sh, or examples/*/e2e.sh (do not execute)"
fi

# --- CLI ---

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

cli_display() {
    if [[ -n "${NITRUM_FN_BIN:-}" ]]; then
        printf '%s' "${NITRUM_FN_BIN}"
    else
        printf 'cargo run -q --bin nitrum-fn'
    fi
}

# --- scaffold / build (nitrum-fn new) ---

# Sets PARENT, NAME, PROJECT, WASM_SRC, TARGET from env defaults.
e2e_init_workspace() {
    PARENT="${NITRUM_FN_E2E_PARENT_DIR:-${REPO_ROOT}/target/nitrum-fn-e2e-workspace}"
    NAME="${NITRUM_FN_E2E_INIT_NAME:-hello-world}"
    PROJECT="${PARENT}/${NAME}"
    TARGET="${TARGET:-wasm32-unknown-unknown}"
    WASM_SRC="${PROJECT}/target/${TARGET}/release/hello_world.wasm"
    export PARENT NAME PROJECT TARGET WASM_SRC
}

step_new() {
    require_var PARENT
    require_var NAME
    require_var PROJECT

    mkdir -p "${PARENT}"
    if [[ -e "${PROJECT}" ]]; then
        log_step "reset ${PROJECT}"
        rm -rf "${PROJECT}"
    fi

    log_step "CLI new ${NAME}"
    log_kv "parent" "${PARENT}"
    (cd "${PARENT}" && cli new "${NAME}") || die "nitrum-fn new failed"
    require_dir "${PROJECT}"
    log_ok "scaffolded ${PROJECT}"
}

# Rewrite the template git runtime dep to a path dep on this checkout.
pin_runtime() {
    require_var PROJECT
    require_var REPO_ROOT

    local cargo_toml="${PROJECT}/Cargo.toml"
    require_file "${cargo_toml}"

    local runtime_path="${REPO_ROOT}/crates/runtime"
    require_dir "${runtime_path}"

    local new_line="runtime = { path = \"${runtime_path}\", package = \"runtime\" }"
    local tmp
    tmp="$(mktemp)"
    while IFS= read -r line || [[ -n "${line}" ]]; do
        if [[ "${line}" == runtime\ =\ \{\ git\ =\ \"https://github.com/matzapata/nitrum-fn\"* ]] \
            || [[ "${line}" == runtime\ =\ \{\ path\ =\ * ]]; then
            printf '%s\n' "${new_line}"
        else
            printf '%s\n' "${line}"
        fi
    done <"${cargo_toml}" >"${tmp}"
    mv "${tmp}" "${cargo_toml}"

    if ! grep -qF "${new_line}" "${cargo_toml}"; then
        die "pin_runtime: failed to rewrite runtime dependency in ${cargo_toml}"
    fi
    log_ok "pinned runtime → ${runtime_path}"
}

step_build_wasm() {
    require_var PROJECT
    require_var TARGET
    require_var WASM_SRC

    require_cmd rustup
    require_cmd cargo

    log_step "build wasm (${TARGET})"
    log_kv "project" "${PROJECT}"
    rustup target add "${TARGET}" >/dev/null
    cargo build --manifest-path "${PROJECT}/Cargo.toml" --target "${TARGET}" --release \
        || die "wasm build failed"
    require_file "${WASM_SRC}"
    log_ok "wasm ${WASM_SRC}"
}

# new → pin → build. Sets HASH from describe.
step_scaffold_and_build() {
    e2e_init_workspace
    step_new
    pin_runtime
    step_build_wasm

    local desc
    log_step "CLI describe"
    desc="$(cli describe "${WASM_SRC}")" || die "describe failed"
    HASH="$(printf '%s\n' "${desc}" | awk -F= '/^hash=/{print $2}')"
    [[ -n "${HASH}" ]] || die "describe missing hash=: ${desc}"
    export HASH
    log_ok "hash=${HASH}"
}

# --- cloud resolve / wait ---

tf_output() {
    terraform -chdir="${TF_DIR}" output -raw "$1"
}

pcr0_from_tfvars() {
    [[ -f "${TFVARS}" ]] || return 0
    sed -nE 's/^[[:space:]]*eif_image_sha384[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/p' "${TFVARS}" | head -1
}

pcr0_norm() {
    local h="${1#0x}"
    h="${h#0X}"
    printf '%s' "${h}" | tr 'A-F' 'a-f'
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
    local host
    host="$(url_host "${url}")"
    if dns_resolves "${host}"; then
        printf '%s' "${url}"
        return 0
    fi

    if [[ "${from_env}" == "1" ]]; then
        log_warn "${label} URL from env does not resolve (${host}); falling back to terraform output ${tf_key}"
        require_cmd terraform
        log_step "resolve terraform output ${tf_key}"
        log_kv "dir" "${TF_DIR}"
        url="$(tf_output "${tf_key}")" || die "terraform output ${tf_key} failed"
        url="${url%/}"
        log_kv "url" "${url}"
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

    require_cmd curl

    log_step "wait ${label}"
    log_kv "timeout" "${timeout_seconds}s"
    log_kv "url" "${url}"

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
            log_ok "${label} ready"
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
        log_wait "http=${code} rc=${rc}  ${remaining}s left"
        sleep_for="${POLL_SECONDS}"
        if ((remaining < POLL_SECONDS)); then
            sleep_for="${remaining}"
        fi
        sleep "${sleep_for}"
    done
}

# Sets API_URL, INVOKE_URL, PCR0 from env and/or terraform. Dies if still missing.
cloud_resolve() {
    require_var TF_DIR
    require_var TFVARS
    require_var API_TIMEOUT
    require_var INVOKE_TIMEOUT
    require_var POLL_SECONDS
    require_var ENCLAVE_TLS_INSECURE

    local api_from_env=0 invoke_from_env=0 pcr0_from_env=0
    local orig_api orig_invoke tf_pcr0
    API_URL="${NITRUM_FN_API_URL:-}"
    INVOKE_URL="${NITRUM_FN_INVOKE_URL:-}"
    PCR0="${NITRUM_FN_PCR0:-}"

    [[ -n "${API_URL}" ]] && api_from_env=1
    [[ -n "${INVOKE_URL}" ]] && invoke_from_env=1
    [[ -n "${PCR0}" ]] && pcr0_from_env=1

    if [[ -z "${API_URL}" || -z "${INVOKE_URL}" ]]; then
        command -v terraform >/dev/null 2>&1 \
            || die "missing NITRUM_FN_API_URL / NITRUM_FN_INVOKE_URL (or install terraform)"
        log_step "resolve terraform outputs"
        log_kv "dir" "${TF_DIR}"
        if [[ -z "${API_URL}" ]]; then
            API_URL="$(tf_output api_url)" || die "terraform output api_url failed"
        fi
        if [[ -z "${INVOKE_URL}" ]]; then
            INVOKE_URL="$(tf_output invoke_url)" || die "terraform output invoke_url failed"
        fi
    fi

    if [[ -z "${PCR0}" ]]; then
        if command -v terraform >/dev/null 2>&1; then
            PCR0="$(tf_output pcr0 2>/dev/null || true)"
            [[ "${PCR0}" == "null" ]] && PCR0=""
        fi
        if [[ -z "${PCR0}" ]]; then
            PCR0="$(pcr0_from_tfvars)"
        fi
    fi

    API_URL="${API_URL%/}"
    INVOKE_URL="${INVOKE_URL%/}"
    orig_api="${API_URL}"
    orig_invoke="${INVOKE_URL}"

    if [[ -z "${API_URL}" || "${API_URL}" == "null" ]]; then
        die "missing API_URL (set NITRUM_FN_API_URL or terraform output api_url)"
    fi
    if [[ -z "${INVOKE_URL}" || "${INVOKE_URL}" == "null" ]]; then
        die "missing INVOKE_URL (set NITRUM_FN_INVOKE_URL or terraform output invoke_url; enable_enclave must be true)"
    fi
    if [[ -z "${PCR0}" ]]; then
        die "missing PCR0 (set NITRUM_FN_PCR0 or terraform output pcr0)"
    fi

    API_URL="$(resolve_endpoint "${API_URL}" "${api_from_env}" api_url api)"
    INVOKE_URL="$(resolve_endpoint "${INVOKE_URL}" "${invoke_from_env}" invoke_url invoke)"

    # Env URLs that NXDOMAIN are from a destroyed stack; env PCR0 is usually the same vintage.
    if [[ "${pcr0_from_env}" == "1" ]] \
        && { [[ "${api_from_env}" == "1" && "${API_URL}" != "${orig_api}" ]] \
            || [[ "${invoke_from_env}" == "1" && "${INVOKE_URL}" != "${orig_invoke}" ]]; }; then
        tf_pcr0=""
        if command -v terraform >/dev/null 2>&1; then
            tf_pcr0="$(tf_output pcr0 2>/dev/null || true)"
            [[ "${tf_pcr0}" == "null" ]] && tf_pcr0=""
        fi
        if [[ -z "${tf_pcr0}" ]]; then
            tf_pcr0="$(pcr0_from_tfvars)"
        fi
        if [[ -n "${tf_pcr0}" && "$(pcr0_norm "${PCR0}")" != "$(pcr0_norm "${tf_pcr0}")" ]]; then
            log_warn "env PCR0 ignored after stale URL fallback; using terraform output pcr0"
            PCR0="${tf_pcr0}"
        fi
    fi

    export API_URL INVOKE_URL PCR0
    export NITRUM_FN_PCR0="${PCR0}"
}

step_wait_api() {
    wait_http "${API_URL}/healthz" "${API_TIMEOUT}" "API"
}

step_wait_enclave() {
    require_cmd curl

    local status_url="${INVOKE_URL}/.well-known/enclave/status"
    local health_url="${INVOKE_URL}/healthz"

    log_step "wait enclave"
    log_kv "timeout" "${INVOKE_TIMEOUT}s"
    log_kv "url" "${INVOKE_URL}"

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
            log_ok "enclave is responsive"
            return 0
        fi
        code_health="$(curl "${curl_args[@]}" "${health_url}")" || rc_health=$?
        if ((rc_health == 0)) && [[ "${code_health}" == "200" ]]; then
            log_ok "enclave is responsive"
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
        log_wait "status=${code_status} health=${code_health}  ${remaining}s left"
        sleep_for="${POLL_SECONDS}"
        if ((remaining < POLL_SECONDS)); then
            sleep_for="${remaining}"
        fi
        sleep "${sleep_for}"
    done
}
