resource "aws_ssm_parameter" "kms_key_id" {
  name        = "/nitrum/${var.project_name}/data-plane/kms_key_id"
  type        = "String"
  value       = aws_kms_key.enclave.key_id
  description = "KMS key ID for Nitrum data-plane (${var.project_name})"
}

resource "aws_ssm_parameter" "dynamodb_table" {
  name        = "/nitrum/${var.project_name}/data-plane/dynamodb_table"
  type        = "String"
  value       = aws_dynamodb_table.enclave.name
  description = "DynamoDB table name for Nitrum data-plane (${var.project_name})"
}
