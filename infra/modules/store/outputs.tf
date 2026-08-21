output "eif_bucket_name" {
  description = "S3 bucket for EIF uploads"
  value       = aws_s3_bucket.eif.bucket
}

output "eif_bucket_arn" {
  description = "ARN of the EIF bucket"
  value       = aws_s3_bucket.eif.arn
}

output "eif_s3_key" {
  description = "Object key the control-plane downloads"
  value       = var.eif_s3_key
}

output "artifacts_bucket_name" {
  description = "S3 bucket for function artifacts (artifacts/{hash}.wasm|.cwasm)"
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

output "publish_topic_arn" {
  description = "SNS topic ARN for publish-queued events"
  value       = aws_sns_topic.publish.arn
}

output "compile_queue_url" {
  description = "SQS queue URL for AOT compile workers"
  value       = aws_sqs_queue.compile.url
}

output "compile_queue_arn" {
  description = "SQS queue ARN for AOT compile workers"
  value       = aws_sqs_queue.compile.arn
}

output "compile_dlq_arn" {
  description = "SQS DLQ ARN for failed compile jobs"
  value       = aws_sqs_queue.compile_dlq.arn
}
