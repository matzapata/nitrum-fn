# Guest WASM fixtures for benches and tests.

- `echo.wat` / `echo.wasm` — minimal v0 ABI (memory + invoke echo; unit tests)
- `hello_world.wasm` — release build of `examples/hello-world` (host-path benches)

Platform e2e (`tests/e2e/local.sh`, `tests/e2e/cloud.sh`) scaffolds via `nitrum-fn new` and builds wasm from source; it does not use this fixture.

Regenerate `echo.wasm` after editing `echo.wat`:

```bash
wat2wasm tests/fixtures/echo.wat -o tests/fixtures/echo.wasm
```

Refresh `hello_world.wasm` after changing the example (for benches):

```bash
cargo build --manifest-path examples/hello-world/Cargo.toml \
  --target wasm32-unknown-unknown --release
cp examples/hello-world/target/wasm32-unknown-unknown/release/hello_world.wasm \
  tests/fixtures/hello_world.wasm
```

Host-path Criterion bench (publish validate / catalog upsert / invoke / Cranelift `runner.run`):

```bash
cargo bench -p executor --bench precompile
```
