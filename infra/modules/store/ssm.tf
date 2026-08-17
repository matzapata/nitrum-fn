resource "aws_ssm_parameter" "store" {
  name        = "/nitrum/${var.project_name}/env/NITRUM_FN_STORE"
  type        = "String"
  value       = "aws"
  description = "nitrum-fn store backend for the enclave host"
}

resource "aws_ssm_parameter" "s3_bucket" {
  name        = "/nitrum/${var.project_name}/env/NITRUM_FN_S3_BUCKET"
  type        = "String"
  value       = aws_s3_bucket.artifacts.bucket
  description = "S3 bucket for nitrum-fn .wasm / .cwasm artifacts"
}

resource "aws_ssm_parameter" "ddb_table" {
  name        = "/nitrum/${var.project_name}/env/NITRUM_FN_DDB_TABLE"
  type        = "String"
  value       = aws_dynamodb_table.catalog.name
  description = "DynamoDB catalog table for nitrum-fn (fn_id + label)"
}
