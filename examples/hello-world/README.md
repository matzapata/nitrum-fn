# Hello-world example

Minimal nitrum-fn guest: no egress, JSON `{"message":"Hello, world!"}`.

`nitrum-fn new` scaffolds from [`crates/cli/template`](../../crates/cli/template) (git `runtime` dep), not this example. This crate path-deps the workspace `runtime` for in-repo demos and fixture refresh.

```bash
# local api + host already running (see CONTRIBUTING.md)
./examples/hello-world/e2e.sh
```

That builds the wasm, deploys it, and invokes with `--fn-shasum`. Override URLs with `NITRUM_FN_API_URL` / `NITRUM_FN_INVOKE_URL` (defaults `http://127.0.0.1:8080` / `8081`). Staging invoke is HTTPS: the script adds `--insecure`; pin PCR0 from `terraform output -raw pcr0` (see [CONTRIBUTING.md](../../CONTRIBUTING.md#cloud-e2e)).

```
examples/hello-world/
  e2e.sh       # build → deploy → invoke
  src/lib.rs   # handler
```

Platform e2e (`tests/e2e/local.sh`, `tests/e2e/cloud.sh`) uses `nitrum-fn new` + a path-pinned `runtime`, not this crate. After changing the example, refresh the bench fixture:

```bash
cargo build --manifest-path examples/hello-world/Cargo.toml \
  --target wasm32-unknown-unknown --release
cp examples/hello-world/target/wasm32-unknown-unknown/release/hello_world.wasm \
  tests/fixtures/hello_world.wasm
```
