#!/usr/bin/env bash
# Build hello-world for wasm32 and install into the host seed directory.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
EXAMPLE="$ROOT/examples/hello-world"
SEED_DIR="${NITRUM_FN_SEED_DIR:-$ROOT/.data/seed}"
TARGET="wasm32-unknown-unknown"

rustup target add "$TARGET" >/dev/null
cargo build --manifest-path "$EXAMPLE/Cargo.toml" --target "$TARGET" --release

WASM_SRC="$EXAMPLE/target/$TARGET/release/hello_world.wasm"
mkdir -p "$SEED_DIR"
cp "$WASM_SRC" "$SEED_DIR/hello-world.wasm"

echo "built $(wc -c < "$WASM_SRC" | tr -d ' ') bytes"
echo "installed $SEED_DIR/hello-world.wasm"
echo "restart the host to register hello-world@latest"
