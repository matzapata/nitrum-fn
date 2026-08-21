variable "project_name" {
  type        = string
  description = "Project slug used in resource names and log groups."

  validation {
    condition     = can(regex("^[a-z][a-z0-9-]{2,127}$", var.project_name))
    error_message = "project_name must match ^[a-z][a-z0-9-]{2,127}$."
  }
}

variable "vpc_id" {
  type        = string
  description = "VPC that also hosts the enclave fleet."
}

variable "public_subnet_ids" {
  type        = list(string)
  description = "Public subnet IDs for the ALB."
}

variable "private_subnet_ids" {
  type        = list(string)
  description = "Private subnet IDs for Fargate tasks."
}

variable "artifacts_bucket_name" {
  type        = string
  description = "S3 bucket for .wasm / .cwasm artifacts."
}

variable "artifacts_bucket_arn" {
  type        = string
  description = "ARN of the artifacts bucket."
}

variable "catalog_table_name" {
  type        = string
  description = "DynamoDB catalog table name."
}

variable "catalog_table_arn" {
  type        = string
  description = "ARN of the catalog table."
}

variable "publish_topic_arn" {
  type        = string
  description = "SNS topic ARN for publish-queued events."
}

variable "image_tag" {
  type        = string
  default     = "latest"
  description = "ECR image tag for nitrum-fn-api."
}

variable "desired_count" {
  type        = number
  default     = 1
  description = "Fargate desired count. Push an image to ECR before raising this above 0 on a fresh stack."
}

variable "cpu" {
  type        = number
  default     = 512
  description = "Fargate task CPU units (API stores .wasm and publishes SNS; compile is off-box)."
}

variable "memory" {
  type        = number
  default     = 1024
  description = "Fargate task memory in MiB."
}

variable "log_retention_in_days" {
  type        = number
  default     = 7
  description = "CloudWatch Logs retention for the API task."
}
