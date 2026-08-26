data "aws_region" "current" {}

resource "aws_ssm_parameter" "env" {
  name        = "/nitrum/${var.project_name}/env/NITRUM_FN_ENV"
  type        = "String"
  value       = "prod"
  description = "Config overlay (config/host/prod.yaml) for the enclave host"
}

resource "aws_ssm_parameter" "s3_bucket" {
  name        = "/nitrum/${var.project_name}/env/NITRUM_FN_ARTIFACTS__BUCKET"
  type        = "String"
  value       = aws_s3_bucket.artifacts.bucket
  description = "S3 bucket for nitrum-fn .wasm / .cwasm artifacts"
}

resource "aws_ssm_parameter" "ddb_table" {
  name        = "/nitrum/${var.project_name}/env/NITRUM_FN_CATALOG__TABLE"
  type        = "String"
  value       = aws_dynamodb_table.catalog.name
  description = "DynamoDB catalog table for nitrum-fn (fn_id + label)"
}

resource "aws_ssm_parameter" "aws_region" {
  name        = "/nitrum/${var.project_name}/env/AWS_REGION"
  type        = "String"
  value       = data.aws_region.current.name
  description = "AWS region for the enclave host SDK (user process env is cleared except SSM + OTel)"
}
