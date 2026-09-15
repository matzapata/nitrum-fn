# Cloud infrastructure

Contributor workflow (local checks, images, staging e2e): [`CONTRIBUTING.md`](../CONTRIBUTING.md).

Staging deploys **network + store + API + publish-worker** by default. The Nitro enclave fleet is optional (`enable_enclave`) so you can publish functions before you have an EIF.

```text
infra/
  modules/
    network/   # VPC, subnets, NAT, S3/DDB/KMS/SSM/Logs/SQS/SNS endpoints
    store/     # EIF S3, artifacts S3, catalog DDB, SNS+SQS, /env SSM
    api/       # HTTP ALB, Fargate (ALB DNS)
    worker/    # Fargate publish-worker (SQS → AOT)
    enclave/   # NLB, ASG, KMS, Nitrum data-plane table (optional)
  envs/staging/
```

TLS for **invoke** terminates in the enclave (NLB TCP passthrough, self-signed with `acme = false`). **Publish** is HTTP to the API ALB DNS (`.wasm` upload + SNS enqueue; AOT is the publish-worker).

`project_name` must equal `[project].name` in `nitrum.toml` (`nitrum-fn`). The data-plane reads `/nitrum/{name}/env/` and `/nitrum/{name}/data-plane/` from that baked-in name. Staging vs prod is a **different AWS account**. Config overlays differ: staging uses `NITRUM_FN_ENV=staging` (`config/shared/staging.yaml`); a future prod env uses `prod`. The `Environment = staging` tag on this env is only for AWS resource tags.

## Prerequisites (once per account)

1. AWS credentials that can create VPC, IAM, ECS, KMS, etc.
2. Terraform state backend:

```bash
# pick unique names
aws s3 mb s3://YOUR_TFSTATE_BUCKET --region us-east-1
aws dynamodb create-table \
  --table-name YOUR_TFSTATE_LOCK \
  --attribute-definitions AttributeName=LockID,AttributeType=S \
  --key-schema AttributeName=LockID,KeyType=HASH \
  --billing-mode PAY_PER_REQUEST \
  --region us-east-1
```

```bash
cd infra/envs/staging
cp backend.hcl.example backend.hcl
# set bucket, dynamodb_table, region (backend.hcl is gitignored)
cp terraform.tfvars.example terraform.tfvars
```

## 1. Network + store + API + worker (no enclaves)

Fargate pulls **public** images. Defaults are GHCR (`ghcr.io/matzapata/nitrum-fn/api:latest` and `…/publish-worker:latest`), published by the [Release workflow](../.github/workflows/release.yml) on `v*` tags. Override `api_image` / `worker_image` in `terraform.tfvars` for Docker Hub or another registry. Make GHCR packages public so ECS can pull without a PAT. Pin to the release tag (not `:latest`) when you want a specific cut.

```bash
cd infra/envs/staging
terraform init -backend-config=backend.hcl
# terraform.tfvars: enable_enclave = false
terraform apply
```

Outputs you need: `api_url`, `api_image`, `worker_image`, `eif_bucket_name`, `artifacts_bucket_name`.

Wait until `http://<alb_dns>/healthz` returns 200 (`terraform output -raw api_url`). Deploy (CLI polls until the worker upserts the catalog):

```bash
cargo run -p cli -- deploy ./path/to/fn.wasm --name hello-world --url "$(terraform -chdir=infra/envs/staging output -raw api_url)"
```

## 2. Enclave fleet (invoke)

Build the EIF with the [Nitrum CLI](https://github.com/matzapata/nitrum) (`nitrum build` uses `./Dockerfile` + `nitrum.toml`). Terraform in this repo owns the stack. Keep `[tls_termination] acme = false` for a self-signed enclave cert. Callers use `curl -k` against the NLB DNS.

```bash
# from repo root (linux/amd64; QEMU on Apple Silicon)
nitrum build
```

`nitrum.toml` `[egress]` is a **whitelist** when `enabled = true`. KMS, SSM, DynamoDB, IMDS, and OTLP are implicit; **S3 is not** — the file already allowlists S3 hostnames so artifact fetches work.

1. Take **PCR0** and **EIF hash (sha256)** from `nitrum build` (or `nitrum describe`).
2. In `terraform.tfvars`:

```hcl
enable_enclave    = true
eif_version_label = "<first 12 hex of EIF sha256>"
eif_image_sha384  = "<PCR0 hex>"
```

3. `terraform apply` — uploads `.nitrum/artifacts/nitrum-fn.eif` to the EIF bucket, then creates NLB, ASG, KMS (PCR0-conditioned), Nitrum data-plane table, and read IAM on the instance role for catalog/artifacts. The ASG waits for the object to exist.

SSM under `/nitrum/<project>/env/` injects `AWS_REGION` and `NITRUM_FN_ENV` so the host SDK and config overlay work after the data-plane clears env. Artifacts bucket, catalog table, and publish-lock table names are literals in `config/shared/{staging,prod}.yaml`; Terraform `yamldecode`s that file to create them, and services read the same values from baked YAML. Account-specific ARNs/URLs (SNS topic, SQS queue) still come from ECS env. Port and prefix live in `config/*.yaml`. Publish (SNS/SQS, lock table) stays on the Fargate API.

**Bucket rename:** changing `artifacts.bucket` in YAML forces S3 replace (name is immutable). With `retain = false` the old bucket is destroyed — republish functions after apply.

Invoke (`curl -k` with the self-signed enclave cert):

```bash
curl -k -X POST "$(terraform -chdir=infra/envs/staging output -raw invoke_url)/invoke/hello-world" \
  -H 'content-type: application/json' -d '{}'
```

Or run the cloud e2e (publish via ALB, invoke via NLB):

```bash
./tests/e2e/cloud.sh
```

## Every enclave release

Prefer the EIF attached to the GitHub Release (`nitrum-fn.eif` + `nitrum-fn.eif.json`). Copy it to `.nitrum/artifacts/nitrum-fn.eif`, bump **both** `eif_version_label` and `eif_image_sha384`, `terraform apply`. Terraform re-uploads the EIF; the launch template name changes and the ASG rolls. `nitrum build` locally only when you want a new PCR0.

## Layout vs trust

| Module | In VPC? | Role |
|---|---|---|
| `network` | is the VPC | Shared by API and enclave |
| `store` | regional | Shared S3 + catalog DDB |
| `api` | private Fargate + public ALB | Publish / catalog (HTTP, ALB DNS) |
| `enclave` | private ASG + public NLB | Invoke only (TLS-in-enclave) |
