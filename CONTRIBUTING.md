# Contributing

Product context: [`README.md`](README.md). Terraform modules: [`infra/README.md`](infra/README.md).

## Prerequisites

- Rust **1.95** (`rust-toolchain.toml`)
- Docker (Compose for Floci; Buildx/QEMU on Apple Silicon for `linux/amd64`)
- Guest examples: `rustup target add wasm32-unknown-unknown`
- Staging / EIF: [Nitrum CLI](https://github.com/matzapata/nitrum) (`nitrum build` / `nitrum describe`), AWS credentials, Terraform ≥ 1.5, and a state backend ([infra README](infra/README.md#prerequisites-once-per-account))

```bash
curl -fsSL https://raw.githubusercontent.com/matzapata/nitrum/develop/scripts/install-nitrum.sh | bash
```

Local checks and `make e2e` do **not** need `nitrum`. Do **not** use `nitrum cloud deploy` — this repo’s Terraform owns the stack. Use `nitrum build` / `describe` only. Keep `[tls_termination] acme = false` in `nitrum.toml` (self-signed invoke TLS; `curl -k` against the NLB).

## Checks

Matches [`.github/workflows/ci.yml`](.github/workflows/ci.yml):


| Target          | What                                            |
| --------------- | ----------------------------------------------- |
| `make format`   | `cargo fmt --all`                               |
| `make check`    | fmt check + Clippy (`-D warnings`)              |
| `make audit`    | `cargo audit`                                   |
| `make test`     | `cargo test --workspace --lib --bins`           |
| `make adapters` | Floci + catalog/artifacts integration tests     |
| `make e2e`      | local publish + invoke (`tests/e2e/local.sh`)   |
| `make ci`       | `check` + `audit` + `test` + `adapters` + `e2e` |


## Local stack

Compose runs **emulators only** (Floci: S3 + SNS + SQS + DynamoDB, plus `aws-init`). **api**, **host**, and **publish-worker** stay off Compose so the inner loop is `cargo run` — incremental compiles, debugger, no image rebuild on every change.

```bash
make stack

export AWS_REGION=us-east-1 AWS_ACCESS_KEY_ID=test AWS_SECRET_ACCESS_KEY=test
cargo run -p publish-worker &
cargo run -p api &
cargo run -p host
```

Then:

```bash
bash examples/hello-world/deploy-local.sh
HASH=$(cargo run -p cli --quiet -- describe \
  examples/hello-world/target/wasm32-unknown-unknown/release/hello_world.wasm \
  | awk -F= '/^hash=/{print $2}')
cargo run -p cli -- invoke hello-world --url http://127.0.0.1:8081 -d '{}' \
  --fn-shasum "$HASH"
```

Automated: `make e2e`. Tear down emulators: `make stack-down`.

## Images


| File                        | Binary                            | Where                             |
| --------------------------- | --------------------------------- | --------------------------------- |
| `Dockerfile`                | data-plane + `nitrum-fn-host`     | EIF via `nitrum build`; GHCR `…/host` |
| `Dockerfile.api`            | `nitrum-fn-api`                   | Fargate / GHCR `…/api`            |
| `Dockerfile.publish-worker` | `nitrum-fn-publish-worker` (musl) | Fargate / GHCR `…/publish-worker` |


```bash
make images            # local linux/amd64 builds of api, worker, and host; no push
make api               # just the API image
make publish-worker    # just the publish-worker image
make host              # just the enclave host image (input to `nitrum build`)

# Override registry/tag per invocation:
make api TAG=sha-1234
make api IMAGE_PREFIX=ghcr.io/you/nitrum-fn TAG=dev
make publish-worker WORKER_IMAGE=docker.io/you/nitrum-fn-worker:custom   # API_IMAGE=... / HOST_IMAGE=...
```

Pushing a `v*` tag runs [`.github/workflows/release.yml`](.github/workflows/release.yml): CI, then GHCR `…/{api,publish-worker,host}:latest` / `:$tag` / `:$sha`, then `nitrum build`. The GitHub Release attaches `nitrum-fn.eif` and `nitrum-fn.eif.json` (sha256 + PCR0). Packages must be **public** so ECS can pull without a PAT. Pin `api_image` / `worker_image` in `terraform.tfvars` to the release tag.

The publish-worker is musl so `.cwasm` matches the enclave. After rolling a new **worker** image, **republish** functions. If Terraform still pins `:latest`, force ECS redeploy:

```bash
aws ecs update-service --cluster nitrum-fn-api --service nitrum-fn-api --force-new-deployment
aws ecs update-service --cluster nitrum-fn-worker --service nitrum-fn-worker --force-new-deployment
```

## Cloud e2e

Not in GitHub Actions. Backend/state detail: [infra/README.md](infra/README.md). URL/PCR0 overrides: [`tests/e2e/.env.example`](tests/e2e/.env.example).

```bash
# 1. EIF (repo root) — download nitrum-fn.eif from a GitHub Release, or build:
nitrum build   # prints EIF sha256 + PCR0 → .nitrum/artifacts/nitrum-fn.eif

# 2. Terraform (API + worker + enclave)
cd infra/envs/staging
cp backend.hcl.example backend.hcl
cp terraform.tfvars.example terraform.tfvars
# set in terraform.tfvars:
#   enable_enclave    = true
#   eif_version_label = "<first 12 hex of EIF sha256>"
#   eif_image_sha384  = "<PCR0>"
terraform init -backend-config=backend.hcl
terraform apply

# 3. Smoke (waits up to ~10m for enclave boot)
make e2e-cloud
```

## Releases

Push a SemVer tag (`vMAJOR.MINOR.PATCH`). The [Release workflow](.github/workflows/release.yml) re-runs CI, publishes GHCR images, builds the EIF, and creates a GitHub Release.

1. **Fargate (api + publish-worker)** — pin `api_image` / `worker_image` to `:$tag`. Force ECS redeploy if the URI is still `:latest`.
2. **Enclave (host)** — download `nitrum-fn.eif` from the release into `.nitrum/artifacts/`, copy `eif_version_label` / `eif_image_sha384` from `nitrum-fn.eif.json` (or the release notes), `terraform apply`. Rebuild with `nitrum build` only when you want a new PCR0.


