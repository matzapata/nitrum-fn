#!/usr/bin/env bash
# Local stand-in for an x402 facilitator. Local development and e2e only.
#
# The API POSTs each signed `exact` payment to `{facilitator_url}/settle` and
# credits the ledger when the reply is `{"success": true}`. This answers every
# request that way, so `nitrum-fn account credit` runs the real x402 path with
# no chain and no funds. It does not check signatures.
#
# Needs BSD or OpenBSD `nc` (macOS default; `netcat-openbsd` on Debian/Ubuntu).
#
# Usage: bash tests/mocks/facilitator.sh        (PORT, default 4402)

set -euo pipefail

PORT="${PORT:-4402}"
BODY='{"success":true,"transaction":"0x0000000000000000000000000000000000000000000000000000000000000000"}'

# Read one request from stdin (headers, then Content-Length bytes) and answer 200.
respond() {
    local line request="" length=0
    while IFS= read -r line; do
        line="${line%$'\r'}"
        [[ -z "${request}" ]] && request="${line}"
        [[ -z "${line}" ]] && break
        if [[ "${line}" =~ ^[Cc]ontent-[Ll]ength:\ *([0-9]+) ]]; then
            length="${BASH_REMATCH[1]}"
        fi
    done
    if ((length > 0)); then
        head -c "${length}" >/dev/null
    fi
    echo "${request}" >&2
    printf 'HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: %d\r\nconnection: close\r\n\r\n%s' \
        "${#BODY}" "${BODY}"
}

FIFO="$(mktemp -u)"
mkfifo "${FIFO}"

serve() {
    while true; do
        nc -l 127.0.0.1 "${PORT}" <"${FIFO}" | respond >"${FIFO}"
    done
}

SERVE_PID=""
stop() {
    if [[ -n "${SERVE_PID}" ]]; then
        pkill -P "${SERVE_PID}" 2>/dev/null || true
        kill "${SERVE_PID}" 2>/dev/null || true
    fi
    rm -f "${FIFO}"
}
trap stop EXIT
trap 'exit 0' TERM INT

echo "mock facilitator listening on 127.0.0.1:${PORT}" >&2
serve &
SERVE_PID=$!
wait "${SERVE_PID}"
