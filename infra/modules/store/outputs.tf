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

output "metrics_log_group_name" {
  description = "CloudWatch log group for EMF metrics (shared by ADOT on enclave and Fargate)"
  value       = aws_cloudwatch_log_group.metrics.name
}

output "metrics_log_group_arn" {
  description = "ARN of the EMF metrics log group"
  value       = aws_cloudwatch_log_group.metrics.arn
}
