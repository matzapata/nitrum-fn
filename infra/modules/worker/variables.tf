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
  description = "VPC that hosts Fargate tasks."
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

variable "publish_lock_table_name" {
  type        = string
  description = "DynamoDB table for per-function publish locks."
}

variable "publish_lock_table_arn" {
  type        = string
  description = "ARN of the publish lock table."
}

variable "compile_queue_url" {
  type        = string
  description = "SQS queue URL for compile jobs."
}

variable "compile_queue_arn" {
  type        = string
  description = "SQS queue ARN for compile jobs."
}

variable "image" {
  type        = string
  description = "Full container image URI for nitrum-fn-publish-worker (registry/repo:tag). Must exist and be pullable by Fargate (public GHCR/Docker Hub, or a registry ECS can authenticate to)."
}

variable "desired_count" {
  type        = number
  default     = 1
  description = "Fargate desired count for the publish worker. The image must exist in the registry before the service can start."
}

variable "cpu" {
  type        = number
  default     = 1024
  description = "Fargate task CPU units (AOT compile needs headroom)."
}

variable "memory" {
  type        = number
  default     = 2048
  description = "Fargate task memory in MiB."
}

variable "log_retention_in_days" {
  type        = number
  default     = 7
  description = "CloudWatch Logs retention for the worker task."
}
