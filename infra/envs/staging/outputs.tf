output "vpc_id" {
  description = "Shared VPC ID"
  value       = module.network.vpc_id
}

output "api_url" {
  description = "HTTP URL of the management API (ALB DNS; publish / catalog)"
  value       = module.api.api_url
}

output "alb_dns_name" {
  description = "API ALB DNS name"
  value       = module.api.alb_dns_name
}

output "artifacts_bucket_name" {
  description = "S3 bucket for function artifacts"
  value       = module.store.artifacts_bucket_name
}

output "catalog_table_name" {
  description = "DynamoDB catalog table"
  value       = module.store.catalog_table_name
}

output "eif_bucket_name" {
  description = "S3 bucket to upload the EIF to"
  value       = module.store.eif_bucket_name
}

output "api_image" {
  description = "Container image URI the API task pulls"
  value       = var.api_image
}

output "worker_image" {
  description = "Container image URI the publish-worker task pulls"
  value       = var.worker_image
}

output "publish_topic_arn" {
  description = "SNS topic for publish-queued events"
  value       = module.store.publish_topic_arn
}

output "compile_queue_url" {
  description = "SQS compile queue URL"
  value       = module.store.compile_queue_url
}

output "nlb_dns_name" {
  description = "Enclave NLB DNS name (null when enable_enclave is false)"
  value       = var.enable_enclave ? module.enclave[0].nlb_dns_name : null
}

output "invoke_url" {
  description = "Invoke HTTPS URL (NLB DNS; self-signed enclave cert — use curl -k)"
  value       = var.enable_enclave ? "https://${module.enclave[0].nlb_dns_name}" : null
}

output "eif_s3_uri" {
  description = "S3 URI the control-plane downloads at startup (null when enable_enclave is false)"
  value       = var.enable_enclave ? module.enclave[0].eif_s3_uri : null
}

output "kms_key_alias_name" {
  description = "KMS alias for the enclave CMK (null when enable_enclave is false)"
  value       = var.enable_enclave ? module.enclave[0].kms_key_alias_name : null
}

output "nitrum_ssm_prefix" {
  description = "Nitrum data-plane SSM prefix (null when enable_enclave is false)"
  value       = var.enable_enclave ? module.enclave[0].nitrum_ssm_prefix : null
}
