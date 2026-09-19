# Contributing

Product context: `[README.md](README.md)`. Terraform modules: `[infra/README.md](infra/README.md)`.

## Prerequisites

- Rust **1.95** (`rust-toolchain.toml`)
- Docker (Compose for Floci; Buildx/QEMU on Apple Silicon for `linux/amd64`)
- Guest examples: `rustup target add wasm32-unknown-unknown`
- Staging / EIF: [Nitrum CLI](https://github.com/matzapata/nitrum) (`nitrum build` / `nitrum describe`), AWS credentials, Terraform ≥ 1.5, and a state backend ([infra README](infra/README.md#prerequisites-once-per-account))

```bash
curl -fsSL https://raw.githubusercontent.com/matzapata/nitrum/develop/scripts/install-nitrum.sh | bash
```

Local checks and `bash tests/e2e/local.sh` do **not** need `nitrum` (local e2e scaffolds via `nitrum-fn new` and builds wasm). Do **not** use `nitrum cloud deploy` — this repo’s Terraform owns the stack. Use `nitrum build` / `describe` only. Keep `[tls_termination] acme = false` in `nitrum.toml` (self-signed invoke TLS; `curl -k` against the NLB).

## Checks

Matches `[.github/workflows/ci.yml](.github/workflows/ci.yml)`:


| Target                    | What                                                |
| ------------------------- | --------------------------------------------------- |
| `make format`             | `cargo fmt --all`                                   |
| `make check`              | fmt check + Clippy (`-D warnings`)                  |
| `make audit`              | `cargo audit`                                       |
| `make test`               | `cargo test --workspace --lib --bins`               |
| `make adapters`           | Floci + catalog/artifacts integration tests         |
| `bash tests/e2e/local.sh` | local publish + invoke                              |
| `make ci`                 | `check` + `audit` + `test` + `adapters` + local e2e |




## Local stack

Compose runs **emulators only** (Floci: S3 + DynamoDB, plus `aws-init`). **api** and **host** stay off Compose so the inner loop is `cargo run` — incremental compiles, debugger, no image rebuild on every change.

```bash
make stack

export AWS_REGION=us-east-1 AWS_ACCESS_KEY_ID=test AWS_SECRET_ACCESS_KEY=test
cargo run -p api &
cargo run -p host
```

Then:

```bash
./examples/hello-world/e2e.sh
```

Automated platform smoke (`nitrum-fn new` → build → deploy → invoke): `bash tests/e2e/local.sh`. Tear down emulators: `make stack-down`.

## Images


| File             | Binary                        | Where                                 |
| ---------------- | ----------------------------- | ------------------------------------- |
| `Dockerfile`     | data-plane + `nitrum-fn-host` | EIF via `nitrum build`; GHCR `…/host` |
| `Dockerfile.api` | `nitrum-fn-api`               | Fargate / GHCR `…/api`                |


```bash
make images            # local linux/amd64 builds of api and host; no push
make api               # just the API image
make host              # just the enclave host image (input to `nitrum build`)

# Override registry/tag per invocation:
make api TAG=sha-1234
make api IMAGE_PREFIX=ghcr.io/you/nitrum-fn TAG=dev
```

Pushing a `v*` tag runs `[.github/workflows/release.yml](.github/workflows/release.yml)`: CI, then parallel jobs `api`, `host`, and `cli`. GHCR remains `…/{api,host}:latest` / `:$tag` / `:$sha`. The GitHub Release attaches `nitrum-fn.eif`, `nitrum-fn.eif.json` (sha256 + PCR0), and CLI binaries (`nitrum-fn-linux-x86_64`, `nitrum-fn-darwin-aarch64`, `nitrum-fn-windows-x86_64.exe`). Packages must be **public** so ECS can pull without a PAT. Pin `api_image` in `terraform.tfvars` to the release tag.

If Terraform still pins `:latest`, force ECS redeploy:

```bash
aws ecs update-service --cluster nitrum-fn-api --service nitrum-fn-api --force-new-deployment
```



## Cloud e2e

Not in GitHub Actions. Backend/state: [infra/README.md](infra/README.md).

Scripts do **not** load `.env`. Copy [`.env.example`](.env.example) once, fill keys, then source it in your shell. After apply recreates the ALB/NLB, unset stale `NITRUM_FN_*` in `.env` (destroyed LB hostnames NXDOMAIN).

From repo root. Tag the API image with the git SHA so ECS rolls. `nitrum build` is the host image — skip `make host`.

```bash
export TF_DIR=infra/envs/staging
export TAG="$(git rev-parse --short HEAD)"
export API_IMAGE="<registry>/<image>:${TAG}"   # must be public; Fargate pulls this

cp .env.example .env   # once
set -a && source .env && set +a
```

### 1. API image — build + push

```bash
make api API_IMAGE="${API_IMAGE}"
docker push "${API_IMAGE}"
echo "${API_IMAGE}"
```

### 2. EIF — `nitrum build`

```bash
nitrum build
```

From the output, copy:

- `EIF hash (sha256): <64 hex>` → first **12** chars = `eif_version_label`
- `PCR0: <96 hex>` → `eif_image_sha384`

File must exist: `.nitrum/artifacts/nitrum-fn.eif`

### 3. Edit `infra/envs/staging/terraform.tfvars`

```hcl
enable_enclave    = true
api_image         = "<paste $API_IMAGE>"
eif_version_label = "<first 12 hex of EIF sha256>"
eif_image_sha384  = "<PCR0 from nitrum build>"
```

Bump **both** `eif_*` fields every rebuild.

```bash
unset NITRUM_FN_API_URL NITRUM_FN_INVOKE_URL NITRUM_FN_PCR0
```

### 4. Apply

First time in this env:

```bash
cp "${TF_DIR}/backend.hcl.example" "${TF_DIR}/backend.hcl"
cp "${TF_DIR}/terraform.tfvars.example" "${TF_DIR}/terraform.tfvars"
# then edit terraform.tfvars as in step 3
terraform -chdir="${TF_DIR}" init -backend-config=backend.hcl
```

```bash
terraform -chdir="${TF_DIR}" apply
```

If you reuse the same `api_image` string, apply is a no-op for ECS — force a roll:

```bash
aws ecs update-service --cluster nitrum-fn-api --service nitrum-fn-api --force-new-deployment
```

### 5. Platform e2e

Resolves `api_url` / `invoke_url` / `pcr0` from terraform outputs unless already set. Waits up to ~10m for enclave boot.

```bash
./tests/e2e/cloud.sh
```

### 6. Hello-world example

This script does not read terraform. Use values from `.env`, or:

```bash
export NITRUM_FN_API_URL="$(terraform -chdir="${TF_DIR}" output -raw api_url)"
export NITRUM_FN_INVOKE_URL="$(terraform -chdir="${TF_DIR}" output -raw invoke_url)"
export NITRUM_FN_PCR0="$(terraform -chdir="${TF_DIR}" output -raw pcr0)"

./examples/hello-world/e2e.sh
```

### 7. Oracle example

Needs `PRIVATE_KEY`, `BASE_SEPOLIA_RPC_URL`, `ORACLE_ADDRESS` in the environment. First time: `git submodule update --init --recursive`. `e2e.sh` runs `setEnclave` for the new PCR0/wasm. Skip env URLs; it falls back to terraform outputs.

```bash
./examples/oracle/e2e.sh
```




## Releases

Push a SemVer tag (`vMAJOR.MINOR.PATCH`). The [Release workflow](.github/workflows/release.yml) re-runs CI, then:


| Job    | Publishes                                                    |
| ------ | ------------------------------------------------------------ |
| `api`  | GHCR `…/api`                                                 |
| `host` | GHCR `…/host` + EIF + measurements                           |
| `cli`  | `nitrum-fn-{linux-x86_64,darwin-aarch64,windows-x86_64.exe}` |


Install the CLI: `curl -fsSL https://raw.githubusercontent.com/matzapata/nitrum-fn/main/scripts/install-nitrum-fn.sh | bash` (or `NITRUM_FN_VERSION=v…`).

1. **Fargate (**`api`**)** — pin `api_image` to `:$tag`. Force ECS redeploy if the URI is still `:latest`.
2. **Host** — download `nitrum-fn.eif` from the release into `.nitrum/artifacts/`, copy `eif_version_label` / `eif_image_sha384` from `nitrum-fn.eif.json` (or the release notes), `terraform apply`. Rebuild with `nitrum build` only when you want a new PCR0.

