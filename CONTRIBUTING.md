# Contributing

How to develop, test, and run a staging e2e. Product context lives in [`README.md`](README.md); Terraform modules in [`infra/README.md`](infra/README.md).

## Prerequisites

- Rust **1.95** (`rust-toolchain.toml`; rustup installs it)
- Docker (Compose for local S3/DynamoDB; Buildx/QEMU on Apple Silicon for `linux/amd64` images)
- `wasm32-unknown-unknown` for guest examples: `rustup target add wasm32-unknown-unknown`
- **[Nitrum CLI](https://github.com/matzapata/nitrum)** for EIF build (`nitrum build`). Staging also needs AWS credentials, Terraform ≥ 1.5, and a state backend (see [infra README](infra/README.md#prerequisites-once-per-account)).

Install the CLI (see [Nitrum’s README](https://github.com/matzapata/nitrum#installation-prebuilt-binary)):

```bash
curl -fsSL https://raw.githubusercontent.com/matzapata/nitrum/develop/scripts/install-nitrum.sh | bash
nitrum --help
```

Local fmt/test/`local.sh` run without `nitrum`. Staging e2e uses `nitrum build` (and `nitrum describe`) for the EIF. Terraform in this repo owns the VPC, Fargate API, and optional enclave fleet.

The API is the ALB hostname over HTTP. Invoke TLS is self-signed in the enclave (`curl -k` against the NLB DNS). Leave `[tls_termination] acme = false` in `nitrum.toml`.

## Checks

CI (`.github/workflows/ci.yml`) runs format, Clippy, `cargo audit`, unit tests (`--lib --bins`), Floci adapter tests (`catalog` / `artifacts` integration tests), and `tests/e2e/local.sh`. Match that locally:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo audit
cargo test --workspace --lib --bins
docker compose up -d --remove-orphans floci
NITRUM_FN_ARTIFACTS__ENDPOINT=http://127.0.0.1:4566 \
NITRUM_FN_CATALOG__ENDPOINT=http://127.0.0.1:4566 \
AWS_REGION=us-east-1 AWS_ACCESS_KEY_ID=test AWS_SECRET_ACCESS_KEY=test \
  cargo test --tests -p catalog -p artifacts
bash tests/e2e/local.sh
```

`local.sh` runs the same split as cloud: **api** (publish/catalog) + **publish-worker** (AOT) + **host** (invoke) against emulators.

## Local stack

Publish is async: the **api** publishes to Floci **SNS**, which fans out to **SQS**; **publish-worker** AOT-compiles; the **host** invokes from the catalog + S3 artifacts. Compose starts Floci (S3+SNS+SQS+DynamoDB) and `aws-init` creates the bucket, tables, topic, and queue (`config/shared/local.yaml`).

```bash
docker compose up -d --remove-orphans floci
docker compose run --rm aws-init

# `config/shared/local.yaml` has Floci/DynamoDB values (`NITRUM_FN_ENV=local` by default).
export AWS_REGION=us-east-1 AWS_ACCESS_KEY_ID=test AWS_SECRET_ACCESS_KEY=test

cargo run -p publish-worker &
cargo run -p api &
cargo run -p host
```

In another terminal:

```bash
bash examples/hello-world/deploy-local.sh
cargo run -p cli -- invoke hello-world --url http://127.0.0.1:8081 -d '{}' \
  --wasm examples/hello-world/target/wasm32-unknown-unknown/release/hello_world.wasm
```

The host responds with `x-nitrum-fn-hash` (sha256 of the `.wasm` it compiled). With `--wasm` / `--expect-hash` the CLI exits non-zero on mismatch. In staging, add `--pcr0 <hex>` so the CLI requires `x-nitrum-fn-attestation` and verifies the Nitro document (`user_data` = wasm hash || sha256(body)). Rebuild the EIF after host changes that affect attestation (enclave uses Nitrum's fixed loopback crypto API when `NITRUM_FN_ENV` is not `local`).

## Staging e2e (cloud)

Two deployables share one VPC by default: **API** (Fargate + HTTP ALB) and **publish-worker** (Fargate, SQS → AOT). The **enclave** (NLB TCP passthrough, invoke) is optional — apply the API/worker first; turn the fleet on after you have an EIF and PCR0.

`nitrum build` always builds `./Dockerfile` as **linux/amd64** (QEMU on Apple Silicon). The API is `Dockerfile.api`; the publish worker is `Dockerfile.publish-worker` (musl, so `.cwasm` matches the enclave).

Fargate pulls **public** images. CI on `main` publishes `ghcr.io/matzapata/nitrum-fn/api` and `ghcr.io/matzapata/nitrum-fn/publish-worker` (those are the Terraform defaults). Make the GHCR packages public (repo → Packages → package settings → Change visibility) so ECS can pull without a PAT. Override `api_image` / `worker_image` in `terraform.tfvars` for Docker Hub or another registry.

### 1. Terraform (API + worker)

```bash
cd infra/envs/staging
cp backend.hcl.example backend.hcl          # bucket, lock table, region; gitignored
cp terraform.tfvars.example terraform.tfvars  # enable_enclave = false
terraform init -backend-config=backend.hcl
terraform apply
```

Wait until `curl "$(terraform -chdir=infra/envs/staging output -raw api_url)/healthz"` returns 200. You can deploy here (CLI polls until the worker catalogs the function); invoke needs the enclave. If you retag `:latest` without changing the image URI, force a new ECS deployment:

```bash
aws ecs update-service --cluster nitrum-fn-api --service nitrum-fn-api --force-new-deployment
aws ecs update-service --cluster nitrum-fn-worker --service nitrum-fn-worker --force-new-deployment
```

### 2. Build the EIF

`nitrum.toml` is copied into the EIF. Keep `acme = false`. From the repo root (`nitrum build` reads `./Dockerfile` + `nitrum.toml`):

```bash
nitrum build
```

That writes `.nitrum/artifacts/nitrum-fn.eif` and prints **EIF hash (sha256)** and **PCR0**. Re-inspect later with `nitrum describe` (see [Nitrum usage](https://github.com/matzapata/nitrum/blob/develop/docs/usage.md)). Terraform uploads the EIF on the next apply (no `aws s3 cp`).

### 3. Enable the fleet

In `infra/envs/staging/terraform.tfvars`:

```hcl
enable_enclave    = true
eif_version_label = "<first 12 hex of EIF sha256 from nitrum build>"
eif_image_sha384  = "<PCR0 from nitrum build>"
```

```bash
terraform -chdir=infra/envs/staging apply
```

The apply uploads `.nitrum/artifacts/nitrum-fn.eif` to the EIF bucket, then creates the NLB/ASG. Every new EIF: `nitrum build`, bump **both** labels, apply (object updates, launch template rolls the ASG).

Artifacts bucket names live in `config/shared/staging.yaml` / `prod.yaml` (`artifacts.bucket`). Terraform creates that exact name; changing it replaces the S3 bucket — with `retain = false` the old one is destroyed, so **republish** functions after apply.

### 4. Cloud smoke

Enclave boot can take several minutes (`cloud.sh` waits up to 10 minutes). From the repo root the script reads `api_url` / `invoke_url` from Terraform and PCR0 from `eif_image_sha384` in `terraform.tfvars` (override with `NITRUM_FN_*` or `tests/e2e/.env`; see `tests/e2e/.env.example`).

```bash
./tests/e2e/cloud.sh
```

That deploys `hello-world` to the ALB, curls `POST /invoke/hello-world` on the NLB (`curl -k`), then runs CLI checks: hash-only `--wasm`, full NSM verify with `--pcr0`, and negatives (wrong hash / wrong PCR0). On failure check ECS (API) or ASG / control-plane logs (enclave).

This job is **not** in GitHub Actions.

## Images

| File | Binary | Where |
|---|---|---|
| `Dockerfile` | data-plane + `nitrum-fn-host` | EIF via `nitrum build` |
| `Dockerfile.api` | `nitrum-fn-api` | Fargate / GHCR `…/api` (store `.wasm`, SNS publish) |
| `Dockerfile.publish-worker` | `nitrum-fn-publish-worker` (musl) | Fargate / GHCR `…/publish-worker` (SQS → AOT `.cwasm`) |

The **publish-worker** must be musl so `.cwasm` matches the enclave. Invoke **only deserializes** `.cwasm`. After rolling a new worker image, **republish** functions so S3 AOT blobs are rebuilt.
