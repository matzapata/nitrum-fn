# Usage

How to write a function, compile it, deploy it, and invoke it against a running `nitrum-fn` deployment.

## Writing a function

A `nitrum-fn` function is a `wasm32-unknown-unknown` crate that depends on `runtime` and exports one handler with `#[runtime::main]`:

```rust
use runtime::{Error, Request};
use serde_json::{json, Value};

#[runtime::main]
fn handler(_req: Request) -> Result<Value, Error> {
    Ok(json!({
        "message": "Hello, world!",
    }))
}
```

```toml
# Cargo.toml
[package]
name = "hello-world"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
runtime = { git = "https://github.com/matzapata/nitrum-fn", package = "runtime" }
serde_json = "1"
```

### Handler signature

```rust
fn handler(req: Request) -> Result<T, Error>
```

- `req: Request` gives you `.method()`, `.path()`, `.headers()` / `.header(name)`, `.body()`, `.body_string()`, and `.json::<T>()`.
- `T` must implement `IntoResponse`. Built-in impls:
  - `serde_json::Value` → `200`, `content-type: application/json`
  - `String` / `&'static str` → `200`, `content-type: text/plain; charset=utf-8`
  - `Vec<u8>` → `200`, `content-type: application/octet-stream`
  - `runtime::Response` → used as-is (for custom status/headers)
- Return `Err(Error)` for failures; the platform converts it into a `500` JSON body `{"error": "..."}`.

Async handlers are not supported yet — `#[runtime::main]` rejects `async fn` at compile time.

### Custom status codes and headers

```rust
use runtime::{Error, Request, Response};

#[runtime::main]
fn handler(req: Request) -> Result<Response, Error> {
    if req.method() != "POST" {
        return Ok(Response::builder().status(405).body("method not allowed").build());
    }
    let body: serde_json::Value = req.json()?;
    Response::builder()
        .status(201)
        .header("x-custom", "yes")
        .json(&body)
}
```

## Compiling

Add the wasm target once, then build in release mode:

```bash
rustup target add wasm32-unknown-unknown
cargo build --target wasm32-unknown-unknown --release
```

The output is `target/wasm32-unknown-unknown/release/<crate_name>.wasm`. It must be under 2 MiB — deploy rejects anything larger.

## Deploying

Use the `nitrum-fn` CLI to publish the compiled `.wasm` to your `nitrum-fn` API:

```bash
nitrum-fn deploy target/wasm32-unknown-unknown/release/hello_world.wasm \
  --name hello-world \
  --url https://api.your-nitrum-fn-deployment.example
```

```
nitrum-fn deploy <WASM_PATH> --name <FUNCTION_NAME> [--url <API_URL>] [--timeout-secs <SECS>]
```

| Flag | Env var | Default | Purpose |
|---|---|---|---|
| `<WASM_PATH>` (positional) | — | — | Path to your compiled `.wasm` module |
| `--name` | — | — | Function name, used later as `/invoke/{name}` |
| `--url` | `NITRUM_FN_URL` | `http://127.0.0.1:8080` | Base URL of your `nitrum-fn` API |
| `--timeout-secs` | `NITRUM_FN_DEPLOY_TIMEOUT_SECS` | `180` | How long to wait for the deploy to finish compiling before giving up |

What it does:

1. Uploads your `.wasm` bytes to the API.
2. Waits until the function is compiled and live, polling until it's ready or the timeout elapses.
3. Prints `deployed {name}@{version} hash={hash} wasm_bytes={n} status=ready` on success.

Deploying again with new code for the same `--name` publishes a new version behind the same `latest` label — invoke calls automatically pick up the new version once it's ready. A `409` response means another deploy for that same function name is already in progress; wait for it to finish and retry.

## Invoking

Once deployed, call your function over HTTPS:

```bash
curl -X POST https://invoke.your-nitrum-fn-deployment.example/invoke/hello-world \
  -H 'content-type: application/json' \
  -d '{"any":"payload"}'
```

- The request method, path, headers, and body are all passed through to your handler as the `Request`.
- The response status, headers, and body come straight from whatever your handler returns.
- Send an `x-nitrum-fn-version` header to target a specific version label instead of `latest` (the default).
- A `404` means the function hasn't finished deploying yet, or the name is wrong.
- Request and response bodies are capped at 1 MiB.

## API reference

| Method | Path | Body | Response |
|---|---|---|---|
| `PUT` | `/functions/{name}` | raw `.wasm` bytes (`content-type: application/wasm`) | `202 {"name","version","hash","wasm_bytes","status":"queued"}`, `409` if a deploy for that name is already in progress, `413` if the wasm is over 2 MiB |
| `GET` | `/functions/{name}` | — | `200 {"name","version","hash"}`, or `404` if never deployed |
| `POST` | `/invoke/{name}` | any body, up to 1 MiB | whatever your handler returns |

The `nitrum-fn deploy` command wraps the two `/functions/{name}` calls into one step; you only need to call them directly if you're scripting deploys yourself instead of using the CLI.
