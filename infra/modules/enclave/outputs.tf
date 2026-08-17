output "project_name" {
  description = "Environment slug (DynamoDB table name, log group path segment, tags)"
  value       = var.project_name
}

output "nitrum_ssm_prefix" {
  description = "Data-plane infra SSM path prefix (/nitrum/{project}/data-plane/)."
  value       = "/nitrum/${var.project_name}/data-plane"
}

output "nlb_dns_name" {
  description = "Network Load Balancer DNS name (TCP 80 and 443 passthrough to the enclave)."
  value       = aws_lb.nlb.dns_name
}

output "nlb_zone_id" {
  description = "Route53 zone ID of the NLB (for alias records)."
  value       = aws_lb.nlb.zone_id
}

output "kms_key_id" {
  description = "KMS key ID"
  value       = aws_kms_key.enclave.key_id
}

output "kms_key_alias_name" {
  description = "KMS alias for operators"
  value       = aws_kms_alias.enclave.name
}

output "dynamodb_table_name" {
  description = "Nitrum data-plane DynamoDB table name"
  value       = aws_dynamodb_table.enclave.name
}

output "instance_role_arn" {
  description = "EC2 instance role ARN"
  value       = aws_iam_role.instance.arn
}

output "instance_role_name" {
  description = "EC2 instance role name (for extra IAM policies at the env root)"
  value       = aws_iam_role.instance.name
}

output "asg_name" {
  description = "Auto Scaling Group name"
  value       = aws_autoscaling_group.nitro.name
}

output "eif_s3_uri" {
  description = "S3 URI the control-plane uses to download the EIF at container startup"
  value       = "s3://${var.eif_s3_bucket}/${var.eif_s3_key}"
}
