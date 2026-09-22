output "eif_bucket_name" {
  description = "S3 bucket for EIF uploads"
  value       = aws_s3_bucket.eif.bucket
}

output "eif_bucket_arn" {
  description = "ARN of the EIF bucket"
  value       = aws_s3_bucket.eif.arn
}

output "artifacts_bucket_name" {
  description = "S3 bucket for function artifacts (artifacts/{hash}.wasm)"
  value       = aws_s3_bucket.artifacts.bucket
}

output "artifacts_bucket_arn" {
  description = "ARN of the artifacts bucket"
  value       = aws_s3_bucket.artifacts.arn
}

output "catalog_table_name" {
  description = "DynamoDB catalog table name"
  value       = aws_dynamodb_table.catalog.name
}

output "catalog_table_arn" {
  description = "ARN of the catalog table"
  value       = aws_dynamodb_table.catalog.arn
}

output "publish_lock_table_name" {
  description = "DynamoDB table for per-function publish locks"
  value       = aws_dynamodb_table.publish_lock.name
}

output "publish_lock_table_arn" {
  description = "ARN of the publish lock table"
  value       = aws_dynamodb_table.publish_lock.arn
}

output "accounts_table_name" {
  description = "DynamoDB invoke-credit accounts table"
  value       = aws_dynamodb_table.accounts.name
}

output "accounts_table_arn" {
  description = "ARN of the accounts table"
  value       = aws_dynamodb_table.accounts.arn
}

output "keys_table_name" {
  description = "DynamoDB bearer-keys table"
  value       = aws_dynamodb_table.keys.name
}

output "keys_table_arn" {
  description = "ARN of the bearer-keys table"
  value       = aws_dynamodb_table.keys.arn
}

output "keys_index_arn" {
  description = "ARN of the keys account_id GSI"
  value       = "${aws_dynamodb_table.keys.arn}/index/account_id_index"
}

output "receipts_table_name" {
  description = "DynamoDB credit-receipt table"
  value       = aws_dynamodb_table.credit_receipts.name
}

output "receipts_table_arn" {
  description = "ARN of the credit-receipt table"
  value       = aws_dynamodb_table.credit_receipts.arn
}

output "metrics_log_group_name" {
  description = "CloudWatch log group for EMF metrics (shared by ADOT on enclave and Fargate)"
  value       = aws_cloudwatch_log_group.metrics.name
}

output "metrics_log_group_arn" {
  description = "ARN of the EMF metrics log group"
  value       = aws_cloudwatch_log_group.metrics.arn
}
