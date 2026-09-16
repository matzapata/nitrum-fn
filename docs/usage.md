# Usage

How to write, configure, deploy, invoke, and (optionally) verify a nitrum-fn function. Platform internals: [architecture.md](architecture.md).

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/matzapata/nitrum-fn/main/scripts/install-nitrum-fn.sh | bash
# pin: NITRUM_FN_VERSION=v0.1.0 bash …
# from source: cargo install --git https://github.com/matzapata/nitrum-fn --locked --bin nitrum-fn
```

In this repo, `cargo run -p cli -- …` is equivalent. Prebuilt assets: `nitrum-fn-linux-x86_64`, `nitrum-fn-darwin-aarch64`, `nitrum-fn-windows-x86_64.exe`.

You ship a `wasm32-unknown-unknown` `cdylib` built with the `runtime` crate. The CLI uploads it to the **management API**. Callers hit **`POST /invoke/{name}`** on the **host** (local HTTP, or HTTPS into a Nitro enclave).

```bash
# mental model
nitrum-fn deploy ./echo.wasm --name echo          # API
nitrum-fn invoke echo -d '{}' --fn-shasum "$HASH"  # host
```

## Write a function

Guest crate (see `examples/hello-world`):

```toml
# Cargo.toml — in this repo, path-dep the workspace runtime (see examples/hello-world).
[package]
name = "hello-world"
edition = "2021"
publish = false

[lib]
crate-type = ["cdylib"]

[dependencies]
runtime = { path = "../../crates/runtime" }
serde_json = "1"

[profile.release]
opt-level = "z"
lto = true
codegen-units = 1
panic = "abort"
strip = true
```

```rust
use runtime::{Error, Request};
use serde_json::{json, Value};

#[runtime::main]
fn handler(_req: Request) -> Result<Value, Error> {
    Ok(json!({ "message": "Hello, world!" }))
}
```

`#[runtime::main]` generates the wasm export `invoke`. The first call registers the handler; there is no separate `_start`.

### Handler contract

| Item | Notes |
| --- | --- |
| Signature | `fn(Request) -> Result<T, Error>` or `async fn(Request) -> Result<T, Error>` |
| `T` | Anything `IntoResponse`: `Response`, `serde_json::Value`, `String`, `&'static str`, `Vec<u8>` |
| `Err` | HTTP 500 `{"error":"..."}` |
| `Request` | Method, path (`/invoke/{name}`), headers, body |
| JSON in | `req.json::<T>()?` or `req.body()` / `req.body_string()?` |

Build a custom response when you need status or headers:

```rust
use runtime::http::{Request, Response};
use runtime::Error;

#[runtime::main]
fn handler(req: Request) -> Result<Response, Error> {
    Ok(Response::builder()
        .status(200)
        .header("content-type", "application/json")
        .body(req.body().to_vec())
        .build())
}
```

Keep successful bodies **canonical** if a chain consumer will `sha256` them (no extra whitespace, stable key order). The oracle example does this on purpose.

### Outbound HTTP

Only **HTTPS GET**, via the host import:

```rust
use runtime::http::{Client, Request, Response};
use runtime::Error;
use serde_json::Value;

#[runtime::main]
async fn handler(_req: Request) -> Result<Response, Error> {
    let upstream = Client::new()
        .get("https://api.example.com/v1/data")
        .send()
        .await?;
    if upstream.status() != 200 {
        return Ok(Response::builder().status(502).body(vec![]).build());
    }
    let data: Value = upstream.json()?;
    Response::builder().json(&data)
}
```

`Client` is wasm32-only. Native `cargo test` of guest crates cannot perform the fetch (the SDK returns an error). Put allowlisted integration behind staging invoke, or stub logic in unit tests.

Denied / timeout / too-large / transport failures surface as `Error` (`http_get: origin not allowed`, etc.).

**Allowlist is required.** An empty list means no egress. Origins are configured at **deploy**, not in Rust. See [Function config](#function-config).

## Function config

The **guest** config is a small TOML (oracle uses `examples/oracle/enclave/nitrum.toml`). It is **not** the repo-root `nitrum.toml` (that file configures the enclave **host**).

```toml
allow_urls = ["https://api.coingecko.com"]
```

Pass it at deploy:

```bash
nitrum-fn deploy ./oracle.wasm --name oracle \
  --config examples/oracle/enclave/nitrum.toml
```

Or flags (repeatable). Flags and file are merged and deduped:

```bash
nitrum-fn deploy ./oracle.wasm --name oracle \
  --allow-url https://api.coingecko.com \
  --allow-url https://example.com
```

Rules:

- HTTPS only. `http://` is rejected.
- Stored as origin `https://host[:port]` — path and query are stripped.
- No localhost, `*.local`, IP literals, or `user:pass@`.
- Max **8** origins.
- At invoke, the request URL’s origin must match exactly (default HTTPS port is `https://host`, not `:443`).

The API receives the list as repeated `x-nitrum-fn-allow-url` headers. Changing allowlist means **redeploy** (new catalog generation).

## Limits

Enforced by the platform (see `crates/domain/src/limits.rs`):

| Limit | Value |
| --- | --- |
| `.wasm` | 2 MiB |
| Invoke body | 1 MiB |
| Guest output | 1 MiB |
| Guest memory | 64 MiB |
| Invoke timeout | 15 s |
| Allowlist size | 8 origins |
| Outbound URL | 2048 bytes |
| Outbound GET body | 256 KiB |
| Outbound GET timeout | 3 s (no redirects) |
| Function name | 1–64 chars: `A–Z a–z 0–9 _ -` |

Oversize wasm or body → `413`. Guest deadline → `504`. Unknown function → `404`. Second publish while the first is compiling → `409`.

## Build

```bash
rustup target add wasm32-unknown-unknown
cargo build --target wasm32-unknown-unknown --release \
  --manifest-path path/to/guest/Cargo.toml
```

Artifact: `target/wasm32-unknown-unknown/release/<crate_name>.wasm` (hyphens in the package name become underscores).

Content hash is sha256 of **those bytes**, not of source. Rebuilds that change a single byte change the hash.

```bash
cargo run -p cli --quiet -- describe ./path/to/fn.wasm
# hash=<64 hex>
# wasm_bytes=<n>
```

Same value as `shasum -a 256` and as response header `x-nitrum-fn-shasum`.

## Deploy

CLI talks to the **API** (`NITRUM_FN_URL`, default `http://127.0.0.1:8080`). It `PUT`s the wasm, then polls `GET /functions/{name}` until the catalog hash matches (default 180s, `NITRUM_FN_DEPLOY_TIMEOUT_SECS`).

```bash
cargo run -p cli -- deploy ./hello_world.wasm --name hello-world
# deployed hello-world@latest hash=… wasm_bytes=… status=ready
```

Staging:

```bash
API_URL="$(terraform -chdir=infra/envs/staging output -raw api_url)"
cargo run -p cli -- deploy ./hello_world.wasm --name hello-world --url "$API_URL"
```

`hello-world` helper: `bash examples/hello-world/deploy-local.sh`.

Ready means the **worker** compiled AOT and upserted the catalog. The host still loads and Cranelift-compiles the verified `.wasm` on invoke (module cache after the first call on that worker).

### HTTP (without the CLI)

```http
PUT /functions/{name}
Content-Type: application/wasm
x-nitrum-fn-allow-url: https://api.example.com
```

```json
{"name":"echo","version":"latest","hash":"…","wasm_bytes":1234,"status":"queued"}
```

```http
GET /functions/{name}
```

```json
{"name":"echo","version":"latest","hash":"…","egress_allow":["https://api.example.com"]}
```

## Invoke

CLI talks to the **host** (`NITRUM_FN_INVOKE_URL`, default `http://127.0.0.1:8081`).

```bash
HASH=$(cargo run -p cli --quiet -- describe ./hello_world.wasm \
  | awk -F= '/^hash=/{print $2}')

cargo run -p cli -- invoke hello-world -d '{}' --fn-shasum "$HASH"
```

Staging enclave (self-signed cert until ACME is on):

```bash
INVOKE_URL="$(terraform -chdir=infra/envs/staging output -raw invoke_url)"
export NITRUM_FN_PCR0="<48-byte PCR0 hex from nitrum build / eif.json>"

cargo run -p cli -- invoke hello-world \
  --url "$INVOKE_URL" --insecure \
  -d '{}' \
  --fn-shasum "$HASH" \
  --pcr0 "$NITRUM_FN_PCR0" \
  --attestation-out attestation.bin
```

| Flag / env | Meaning |
| --- | --- |
| `-d` / `--data` | JSON body (default `{}`) |
| `--url` / `NITRUM_FN_INVOKE_URL` | Host base URL |
| `--insecure` | Accept self-signed TLS |
| `--version` | Catalog label (default `latest`) |
| `--fn-shasum` | Require `x-nitrum-fn-shasum` to match |
| `--pcr0` / `NITRUM_FN_PCR0` | Verify Nitro document (requires `--fn-shasum`) |
| `--attestation-out FILE` | Write raw COSE Sign1 bytes (requires `--pcr0`; stdout stays the body) |

`--pcr0` always sends a fresh 32-byte nonce as `x-nitrum-fn-nonce`. Local host has no NSM: omit `--pcr0`.

### HTTP

```http
POST /invoke/{name}
Content-Type: application/json
x-nitrum-fn-version: latest
x-nitrum-fn-nonce: <base64 16–32 bytes>
```

Success headers:

- `x-nitrum-fn-shasum` — sha256 of the wasm compiled for this call
- `x-nitrum-fn-attestation` — base64 COSE Sign1, only if nonce was sent **and** the host is in-enclave

Guest status and headers are forwarded. curl against a self-signed NLB: `curl -k`.

## Pin which wasm ran

Three independent checks, use as many as you need:

1. **`describe` / `shasum -a 256`** — hash of the file you uploaded.
2. **`x-nitrum-fn-shasum`** — hash of the bytes the host compiled (CLI `--fn-shasum` compares them).
3. **Attestation `user_data[0:32]`** — same hash, signed by the Nitro NSM, bound to PCR0.

That proves the **artifact you uploaded**, not a fresh local rebuild of source.

## On-chain verification

Trust is not a Nitrum public key. A Solidity consumer checks AWS Nitro PKI ([`base/nitro-validator`](https://github.com/base/nitro-validator)), then policy:

| Pin | What | How you get it |
| --- | --- | --- |
| PCR0 | EIF / enclave image | `nitrum build` / `nitrum-fn.eif.json` / Terraform `eif_image_sha384` |
| Content hash | Guest `.wasm` | `nitrum-fn describe` / `x-nitrum-fn-shasum` |
| Body hash | HTTP response bytes | `sha256(body)` must equal `user_data[32:64]` |

On-chain, PCR0 is stored as `keccak256(raw 48-byte PCR0)`. Content hash is the 32-byte sha256 — **do not call this PCR1** (AWS PCR1 is a different measurement).

Worked example: `examples/oracle` (CoinGecko guest + `NitrumOracle` on Base Sepolia). Demo: [`docs/assets/oracle-demo.mp4`](assets/oracle-demo.mp4).

### 1. Guest body the contract can rebuild

Successful oracle responses are compact JSON, USD × 1e8:

```json
{"ids":["eth"],"prices":[350012000000]}
```

Host `user_data` = `sha256(oracle.wasm) || sha256(that body)`. The contract parses, re-encodes, and requires `keccak256(rebuilt) == keccak256(body)` so the posted bytes are the attested bytes.

### 2. Deploy verifier + oracle (once)

```bash
cd examples/oracle/contracts
export PRIVATE_KEY=0x…
export BASE_SEPOLIA_RPC_URL=https://sepolia.base.org

forge script script/Deploy.s.sol:Deploy \
  --rpc-url "$BASE_SEPOLIA_RPC_URL" --broadcast
```

Prints `NitrumOracle`. Optional `EXISTING_VALIDATOR` reuses a shared `NitroValidator`. `MAX_AGE_SECONDS` defaults to 3600.

### 3. Pin the enclave

```bash
WASM=./examples/oracle/enclave/target/wasm32-unknown-unknown/release/oracle.wasm
HASH=$(cargo run -p cli --quiet -- describe "$WASM" | awk -F= '/^hash=/{print $2}')

export ORACLE_ADDRESS=0x…
export PCR0=<48-byte hex, no 0x>
export CONTENT_HASH="0x$HASH"

forge script script/SetEnclave.s.sol:SetEnclave \
  --rpc-url "$BASE_SEPOLIA_RPC_URL" --broadcast
```

```solidity
oracle.setEnclave(pcr0Hash, contentHash);
// pcr0Hash    = keccak256(48-byte PCR0)
// contentHash = sha256(oracle.wasm)
```

`PCR0_HASH=0x…` skips the keccak if you already have the on-chain `bytes32`.

Redeploying the **guest** (new wasm hash) or the **EIF** (new PCR0) requires `setEnclave` again.

### 4. Deploy the function and take an attested invoke

```bash
cargo build --manifest-path examples/oracle/enclave/Cargo.toml \
  --target wasm32-unknown-unknown --release

cargo run -p cli -- deploy "$WASM" --name oracle \
  --url "$API_URL" \
  --config examples/oracle/enclave/nitrum.toml

cargo run -p cli -- invoke oracle \
  --url "$INVOKE_URL" --insecure \
  -d '{"ids":["eth"]}' \
  --fn-shasum "$HASH" \
  --pcr0 "$PCR0" \
  --attestation-out attestation.bin
```

`--attestation-out` writes the verified COSE document (do not grep stderr for it). Local invoke has **no** document.

### 5. Submit on-chain

`NitrumOracle.updatePrice(tbs, signature, hints, body)`:

1. [`NitroValidator`](https://github.com/base/nitro-validator) — COSE + AWS Nitro root (via `CertManager`).
2. Policy — PCR0, freshness (`maxAge`, monotonic timestamp), content hash, body hash.
3. `CanonicalPriceJson` — parse/encode round-trip, then store prices.

First time a leaf appears, cache the cabundle (`CacheCerts`). Warm path is signature-only.

```bash
# hex without stray newlines (BodyHashMismatch if you keep a trailing \n)
export ATTESTATION_HEX=0x$(xxd -p -c 256 attestation.bin | tr -d '\n')
export BODY_HEX=0x$(printf '%s' '{"ids":["eth"],"prices":[...]}' | xxd -p -c 256 | tr -d '\n')

# cold: cabundle + leaf (under-estimated gas on public RPCs)
forge script script/UpdatePrices.s.sol:CacheCerts \
  --rpc-url "$BASE_SEPOLIA_RPC_URL" --broadcast --ffi --offline \
  --gas-estimate-multiplier 200

# warm: write calldata, then send with a hard gas cap
forge script script/UpdatePrices.s.sol:UpdatePrices --ffi --offline
cast send "$ORACLE_ADDRESS" "$(cat updatePrice.data)" \
  --rpc-url "$BASE_SEPOLIA_RPC_URL" --private-key "$PRIVATE_KEY" \
  --gas-limit 16000000
```

`--ffi` needs Node (`p384_hints.js`). `--offline` avoids Sourcify hangs. Prefer a dedicated Base Sepolia RPC; public endpoints often reject ~16M-gas txs. Do not broadcast `updatePrice` through Foundry — local `modexp` metering exceeds typical caps.

### 6. Read

```solidity
oracle.getPrice("eth");              // USD × 1e8
oracle.getPriceData("eth");          // price + attestation timestamp ms
```

`350012000000` → $3500.12.

### Policy gotchas

- Do not use attestation **signature** bytes as uniqueness keys (P-384 malleability). Use NSM timestamp + content.
- Debug / zero PCR0 is rejected (`DebugPcr0`).
- Stale or out-of-order timestamps revert.
- Production `CertManager` owner / revoker should be multisigs.

Full operator notes: [`examples/oracle/README.md`](../examples/oracle/README.md).

## CLI cheat sheet

```bash
nitrum-fn --help
nitrum-fn deploy --help
nitrum-fn invoke --help

# in this repo:
cargo run -p cli -- --help
```

| Env | Used by |
| --- | --- |
| `NITRUM_FN_URL` | `deploy` API base (default `http://127.0.0.1:8080`) |
| `NITRUM_FN_DEPLOY_TIMEOUT_SECS` | Poll for catalog ready (default `180`) |
| `NITRUM_FN_INVOKE_URL` | `invoke` host base (default `http://127.0.0.1:8081`) |
| `NITRUM_FN_PCR0` | Invoke PCR0 pin |

## Examples

| Path | What |
| --- | --- |
| `examples/hello-world` | JSON `{"message":"Hello, world!"}`, no egress |
| `examples/oracle/enclave` | Allowlisted CoinGecko GET, canonical body |
| `examples/oracle/contracts` | Foundry consumer of the attested body |

Local stack (API + worker + host): [CONTRIBUTING.md](../CONTRIBUTING.md). End-to-end: `make e2e` / `./tests/e2e/cloud.sh`.
