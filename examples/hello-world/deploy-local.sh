#!/usr/bin/env bash
# Build hello-world for wasm32 and deploy it to a running nitrum-fn API.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
EXAMPLE="$ROOT/examples/hello-world"
TARGET="wasm32-unknown-unknown"
URL="${NITRUM_FN_URL:-http://127.0.0.1:8080}"

rustup target add "$TARGET" >/dev/null
cargo build --manifest-path "$EXAMPLE/Cargo.toml" --target "$TARGET" --release

WASM_SRC="$EXAMPLE/target/$TARGET/release/hello_world.wasm"

echo "built $(wc -c < "$WASM_SRC" | tr -d ' ') bytes"
echo "deploying hello-world to $URL (api must be running)"
cargo run --manifest-path "$ROOT/Cargo.toml" -p cli --quiet -- deploy "$WASM_SRC" --name hello-world --url "$URL"
