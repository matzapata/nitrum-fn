#!/usr/bin/env bash
# Invoke staging oracle with attestation, write capture/{attestation,body}.hex for UpdatePrices.s.sol.
#
# Requires a real Nitro host (local NoopAttestor does not mint documents).
#
# Env:
#   INVOKE_URL   enclave invoke base URL (https://…)
#   PCR0         hex PCR0 from nitrum build / eif.json
#   OUT_DIR      default: examples/oracle/contracts/capture
#   IDS          default: eth
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../../../.." && pwd)"
ENCLAVE="$ROOT/examples/oracle/enclave"
OUT_DIR="${OUT_DIR:-$ROOT/examples/oracle/contracts/capture}"
INVOKE_URL="${INVOKE_URL:?set INVOKE_URL}"
PCR0="${PCR0:?set PCR0}"
IDS="${IDS:-eth}"

WASM="$ENCLAVE/target/wasm32-unknown-unknown/release/oracle.wasm"
if [[ ! -f "$WASM" ]]; then
  echo "building oracle.wasm"
  rustup target add wasm32-unknown-unknown >/dev/null
  cargo build --manifest-path "$ENCLAVE/Cargo.toml" --target wasm32-unknown-unknown --release
fi

mkdir -p "$OUT_DIR"

set +e
OUT="$(
  cargo run --manifest-path "$ROOT/Cargo.toml" -p cli --quiet -- invoke oracle \
    --url "$INVOKE_URL" --insecure \
    -d "{\"ids\":[\"$IDS\"]}" \
    --wasm "$WASM" \
    --pcr0 "$PCR0" \
    2>"$OUT_DIR/invoke.stderr"
)"
STATUS=$?
set -e
printf '%s' "$OUT" >"$OUT_DIR/body.json"

if [[ $STATUS -ne 0 ]]; then
  echo "invoke failed:" >&2
  cat "$OUT_DIR/invoke.stderr" >&2
  exit "$STATUS"
fi

grep -E '^[[:space:]]*document=' "$OUT_DIR/invoke.stderr" | tail -1 \
  | sed 's/^[[:space:]]*document=//' >"$OUT_DIR/attestation.b64"
if [[ ! -s "$OUT_DIR/attestation.b64" ]]; then
  echo "no attestation document in CLI stderr; is this a real enclave?" >&2
  cat "$OUT_DIR/invoke.stderr" >&2
  exit 1
fi

python3 - <<PY
import base64, pathlib
out = pathlib.Path("$OUT_DIR")
doc = base64.b64decode(out.joinpath("attestation.b64").read_text().strip())
body = out.joinpath("body.json").read_bytes()
out.joinpath("attestation.hex").write_text("0x" + doc.hex() + "\n")
out.joinpath("body.hex").write_text("0x" + body.hex() + "\n")
print(f"wrote {out}/attestation.hex ({len(doc)} bytes)")
print(f"wrote {out}/body.hex ({len(body)} bytes)")
print(f"body: {body.decode()}")
PY

echo
echo "Next:"
echo "  cd $ROOT/examples/oracle/contracts"
echo "  export ORACLE_ADDRESS=0x…"
echo "  forge script script/UpdatePrices.s.sol:UpdatePrices --rpc-url \"\$BASE_SEPOLIA_RPC_URL\" --broadcast --ffi"
