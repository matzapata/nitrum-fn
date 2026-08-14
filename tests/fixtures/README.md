# Guest WASM fixtures for benches and tests.

- `echo.wat` / `echo.wasm` — minimal v0 ABI (memory + invoke echo; unit tests)
- `hello_world.wasm` — release build of `examples/hello-world` (host-path benches)

Regenerate `echo.wasm` after editing `echo.wat`:

```bash
wat2wasm tests/fixtures/echo.wat -o tests/fixtures/echo.wasm
```

Refresh `hello_world.wasm` after changing the example:

```bash
cargo build --manifest-path examples/hello-world/Cargo.toml \
  --target wasm32-unknown-unknown --release
cp examples/hello-world/target/wasm32-unknown-unknown/release/hello_world.wasm \
  tests/fixtures/hello_world.wasm
```

Host-path Criterion bench (publish / invoke wasm vs `.cwasm`):

```bash
cargo bench -p executor --bench precompile
```

No in-process Module cache — each invoke reloads from artifacts (see ARCHITECTURE.md §9).
