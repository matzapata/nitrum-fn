# Cloud infrastructure

Staging deploys **network + store + API** by default. The Nitro enclave fleet is optional (`enable_enclave`) so you can publish functions before you have an EIF.

```text
infra/
  modules/
    network/   # VPC, subnets, NAT, S3/DDB/KMS/SSM/Logs endpoints
    store/     # EIF bucket, artifacts bucket, catalog table, /env SSM
    api/       # ACM, ALB, ECR, Fargate
    enclave/   # NLB, ASG, KMS, Nitrum data-plane table (optional)
  envs/staging/
```

TLS for **invoke** still terminates only in the enclave (NLB TCP passthrough). TLS for **publish** terminates at the API ALB (metadata + `.wasm` only).

`project_name` must equal `[project].name` in `nitrum.toml` (`nitrum-fn`). The data-plane reads `/nitrum/{name}/env/` and `/nitrum/{name}/data-plane/` from that baked-in name. Staging vs prod is a **different AWS account** (and `api_hostname` / `invoke_hostname`), not a second slug. The `Environment = staging` tag on this env is only for AWS resource tags.

## Prerequisites (once per account)

1. AWS credentials that can create VPC, IAM, ECS, KMS, etc.
2. A Route53 **hosted zone** you already own (this stack does not create the zone).
3. Terraform state backend:

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
# set bucket, dynamodb_table, region
cp terraform.tfvars.example terraform.tfvars
# set hosted_zone_id, api_hostname
```

## 1. Network + store + API (no enclaves)

```bash
cd infra/envs/staging
terraform init -backend-config=backend.hcl
# terraform.tfvars: enable_enclave = false
terraform apply
```

Outputs you need: `ecr_repository_url`, `api_url`, `eif_bucket_name`, `artifacts_bucket_name`.

Build and push the management API (Fargate stays unhealthy until this exists):

```bash
# from repo root; region/account from your AWS profile
AWS_REGION=us-east-1
ECR=$(terraform -chdir=infra/envs/staging output -raw ecr_repository_url)
aws ecr get-login-password --region "$AWS_REGION" | docker login --username AWS --password-stdin "${ECR%%/*}"
docker build -t "$ECR:latest" .
docker push "$ECR:latest"
```

Wait until `https://<api_hostname>/healthz` returns 200. Publish:

```bash
cargo run -p cli -- publish ./path/to/fn.wasm --name hello-world --url "$(terraform -chdir=infra/envs/staging output -raw api_url)"
```

## 2. Enclave fleet (invoke)

The Nitrum CLI's `nitrum build` always runs `docker build -f Dockerfile`. That file is the Fargate API image. Build the EIF from `Dockerfile.enclave` instead (data-plane + `/app/nitrum-fn-host`, `CMD` starts the data-plane with `/app/nitrum.toml`).

```bash
# from repo root (linux/amd64; QEMU on Apple Silicon)
DATA_PLANE=$(awk -F '"' '/^data_plane / { print $2; exit }' nitrum.toml)
NITRO_CLI=$(awk -F '"' '/^nitro_cli / { print $2; exit }' nitrum.toml)

docker build --platform linux/amd64 -f Dockerfile.enclave \
  --build-arg "DATA_PLANE_IMAGE=$DATA_PLANE" \
  -t nitrum-fn:enclave .

mkdir -p .nitrum/artifacts
docker run --rm --platform linux/amd64 \
  -v /var/run/docker.sock:/var/run/docker.sock \
  -v "$PWD/.nitrum/artifacts:/output" \
  "$NITRO_CLI" \
  build-enclave --docker-uri nitrum-fn:enclave --output-file /output/nitrum-fn.eif

docker run --rm --platform linux/amd64 \
  -v "$PWD/.nitrum/artifacts:/eif" \
  "$NITRO_CLI" \
  describe-eif --eif-path /eif/nitrum-fn.eif
```

`nitrum.toml` `[egress]` is a **whitelist** when `enabled = true`. KMS, SSM, DynamoDB, IMDS, and OTLP are implicit; **S3 is not** — the file already allowlists S3 hostnames so artifact fetches work.

1. Upload to the EIF bucket (`terraform output eif_bucket_name`, key `enclave.eif` by default):

```bash
EIF_BUCKET=$(terraform -chdir=infra/envs/staging output -raw eif_bucket_name)
aws s3 cp .nitrum/artifacts/nitrum-fn.eif "s3://$EIF_BUCKET/enclave.eif"
```

2. Take PCR0 from `describe-eif` (`Measurements.PCR0`) and a short sha256 of the file (`shasum -a 256 .nitrum/artifacts/nitrum-fn.eif`).
3. In `terraform.tfvars`:

```hcl
enable_enclave    = true
eif_version_label = "<first 12 hex of EIF sha256>"
eif_image_sha384  = "<PCR0 hex>"
invoke_hostname   = "fn.staging.example.com"  # optional NLB alias
```

4. `terraform apply` — creates NLB, ASG, KMS (PCR0-conditioned), Nitrum data-plane table, and read IAM on the instance role for catalog/artifacts.

SSM under `/nitrum/<project>/env/` already has `NITRUM_FN_STORE`, `NITRUM_FN_S3_BUCKET`, and `NITRUM_FN_DDB_TABLE` so Nitrum can inject them into the enclave.

Invoke:

```bash
curl -X POST "https://fn.staging.example.com/invoke/hello-world" \
  -H 'content-type: application/json' -d '{}'
```

(Use the NLB DNS from `nlb_dns_name` if you skipped `invoke_hostname`.)

## Every enclave release

Upload a new EIF, bump **both** `eif_version_label` and `eif_image_sha384`, `terraform apply`. The launch template name changes and the ASG rolls.

## Layout vs trust

| Module | In VPC? | Role |
|---|---|---|
| `network` | is the VPC | Shared by API and enclave |
| `store` | regional | Shared S3 + catalog DDB |
| `api` | private Fargate + public ALB | Publish / catalog |
| `enclave` | private ASG + public NLB | Invoke only |
