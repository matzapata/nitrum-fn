# Contributing

How to develop, test, and run a staging e2e. Product context lives in [`README.md`](README.md); Terraform modules in [`infra/README.md`](infra/README.md).

## Prerequisites

- Rust **1.95** (`rust-toolchain.toml`; rustup installs it)
- Docker (Compose for local S3/DynamoDB; Buildx/QEMU on Apple Silicon for `linux/amd64` images)
- AWS CLI (create bucket/tables/topic/queue against Floci + DynamoDB Local; Terraform owns cloud)
- `wasm32-unknown-unknown` for guest examples: `rustup target add wasm32-unknown-unknown`
- **[Nitrum CLI](https://github.com/matzapata/nitrum)** for EIF build (`nitrum build`). Staging also needs AWS credentials, Terraform ≥ 1.5, and a state backend (see [infra README](infra/README.md#prerequisites-once-per-account)).

Install the CLI (see [Nitrum’s README](https://github.com/matzapata/nitrum#installation-prebuilt-binary)):

```bash
curl -fsSL https://raw.githubusercontent.com/matzapata/nitrum/develop/scripts/install-nitrum.sh | bash
nitrum --help
```

Local fmt/test/`local.sh` do not need `nitrum`. Staging e2e does. Do **not** use `nitrum cloud deploy` here — this repo’s Terraform owns the VPC, Fargate API, and optional enclave fleet. Use `nitrum build` (and `nitrum describe`) only.

No custom DNS. The API is the ALB hostname over HTTP. Invoke TLS is self-signed in the enclave (`curl -k` against the NLB DNS). Leave `[tls_termination] acme = false` in `nitrum.toml`.

## Checks

CI (`.github/workflows/ci.yml`) runs format, Clippy, `cargo audit`, workspace tests (with Floci S3+SNS+SQS + DynamoDB Local), and `tests/e2e/local.sh`. Match that locally:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo audit
docker compose up -d
NITRUM_FN_ARTIFACTS__ENDPOINT=http://127.0.0.1:4566 \
NITRUM_FN_CATALOG__ENDPOINT=http://127.0.0.1:8000 \
AWS_REGION=us-east-1 AWS_ACCESS_KEY_ID=test AWS_SECRET_ACCESS_KEY=test \
  cargo test --workspace --all-targets
bash tests/e2e/local.sh
```

`local.sh` runs the same split as cloud: **api** (publish/catalog) + **publish-worker** (AOT) + **host** (invoke) against emulators.

## Local stack

Publish is async: the **api** publishes to Floci **SNS**, which fans out to **SQS**; **publish-worker** AOT-compiles; the **host** invokes from the catalog + S3 artifacts. Start emulators (Floci + DynamoDB Local), create the bucket/tables/topic/queue, then api + worker + host:

```bash
docker compose up -d

export AWS_REGION=us-east-1 AWS_ACCESS_KEY_ID=test AWS_SECRET_ACCESS_KEY=test
aws --endpoint-url http://127.0.0.1:4566 s3 mb s3://nitrum-fn
aws --endpoint-url http://127.0.0.1:8000 dynamodb create-table \
  --table-name nitrum-fn-catalog \
  --attribute-definitions AttributeName=fn_id,AttributeType=S AttributeName=label,AttributeType=S \
  --key-schema AttributeName=fn_id,KeyType=HASH AttributeName=label,KeyType=RANGE \
  --billing-mode PAY_PER_REQUEST
aws --endpoint-url http://127.0.0.1:8000 dynamodb create-table \
  --table-name nitrum-fn-catalog-idempotency \
  --attribute-definitions AttributeName=idempotency_key,AttributeType=S \
  --key-schema AttributeName=idempotency_key,KeyType=HASH \
  --billing-mode PAY_PER_REQUEST
aws --endpoint-url http://127.0.0.1:8000 dynamodb update-time-to-live \
  --table-name nitrum-fn-catalog-idempotency \
  --time-to-live-specification Enabled=true,AttributeName=expires_at
aws --endpoint-url http://127.0.0.1:4566 sqs create-queue \
  --queue-name nitrum-fn-compile \
  --attributes VisibilityTimeout=300,ReceiveMessageWaitTimeSeconds=20
QUEUE_ARN=$(aws --endpoint-url http://127.0.0.1:4566 sqs get-queue-attributes \
  --queue-url http://127.0.0.1:4566/000000000000/nitrum-fn-compile \
  --attribute-names QueueArn --query Attributes.QueueArn --output text)
TOPIC_ARN=$(aws --endpoint-url http://127.0.0.1:4566 sns create-topic \
  --name nitrum-fn-publish --query TopicArn --output text)
aws --endpoint-url http://127.0.0.1:4566 sns subscribe \
  --topic-arn "$TOPIC_ARN" --protocol sqs \
  --notification-endpoint "$QUEUE_ARN" \
  --attributes RawMessageDelivery=true

# `config/{host,worker,api}/local.yaml` has Floci/DynamoDB values (`NITRUM_FN_ENV=local` by default).
export AWS_REGION=us-east-1 AWS_ACCESS_KEY_ID=test AWS_SECRET_ACCESS_KEY=test

cargo run -p publish-worker &
cargo run -p api &
cargo run -p host
```

In another terminal:

```bash
bash examples/hello-world/deploy-local.sh
curl -X POST http://127.0.0.1:8081/invoke/hello-world \
  -H 'content-type: application/json' -d '{}'
```

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

### 4. Cloud smoke

Enclave boot can take several minutes (`cloud.sh` waits up to 10 minutes).

```bash
export NITRUM_FN_API_URL="$(terraform -chdir=infra/envs/staging output -raw api_url)"
export NITRUM_FN_INVOKE_URL="$(terraform -chdir=infra/envs/staging output -raw invoke_url)"
bash tests/e2e/cloud.sh
```

That deploys `hello-world` to the ALB and `POST /invoke/hello-world` on the NLB (`curl -k`). Success body: `{"message":"Hello, world!"}`. On failure the script prints both URLs; check ECS (API) or ASG / control-plane logs (enclave).

This job is **not** in GitHub Actions.

## Images

| File | Binary | Where |
|---|---|---|
| `Dockerfile` | data-plane + `nitrum-fn-host` | EIF via `nitrum build` |
| `Dockerfile.api` | `nitrum-fn-api` | Fargate / GHCR `…/api` (store `.wasm`, SNS publish) |
| `Dockerfile.publish-worker` | `nitrum-fn-publish-worker` (musl) | Fargate / GHCR `…/publish-worker` (SQS → AOT `.cwasm`) |

The **publish-worker** must be musl so `.cwasm` matches the enclave. Invoke **only deserializes** `.cwasm`. After rolling a new worker image, **republish** functions so S3 AOT blobs are rebuilt.
