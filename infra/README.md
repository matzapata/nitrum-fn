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

1. Build the Nitrum EIF for `nitrum-fn-host`. Allow S3 and DynamoDB in `[egress]` (VPC endpoints still need the data-plane allowlist).
2. Upload to the EIF bucket (`terraform output eif_bucket_name`, key `enclave.eif` by default).
3. Read PCR0: `nitro-cli describe-eif --eif-path enclave.eif`.
4. In `terraform.tfvars`:

```hcl
enable_enclave    = true
eif_version_label = "<first 12 hex of EIF sha256>"
eif_image_sha384  = "<PCR0 hex>"
invoke_hostname   = "fn.staging.example.com"  # optional NLB alias
```

5. `terraform apply` — creates NLB, ASG, KMS (PCR0-conditioned), Nitrum data-plane table, and read IAM on the instance role for catalog/artifacts.

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
