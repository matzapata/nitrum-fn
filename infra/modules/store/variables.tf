variable "project_name" {
  type        = string
  description = "Project slug used in bucket/table names and SSM paths."

  validation {
    condition     = can(regex("^[a-z][a-z0-9-]{2,127}$", var.project_name))
    error_message = "project_name must match ^[a-z][a-z0-9-]{2,127}$."
  }
}

variable "artifacts_bucket_name" {
  type        = string
  description = "S3 artifacts bucket name from config/shared/{run_env}.yaml (artifacts.bucket)."
}

variable "catalog_table_name" {
  type        = string
  description = "DynamoDB catalog table name from config/shared/{run_env}.yaml (catalog.table)."
}

variable "publish_lock_table_name" {
  type        = string
  description = "DynamoDB publish-lock table name from config/shared/{run_env}.yaml (catalog.publish_lock_table)."
}

variable "run_env" {
  type        = string
  description = "NITRUM_FN_ENV overlay name reinjected via SSM for the enclave host (e.g. staging, prod)."
}

variable "retain" {
  type        = bool
  default     = false
  description = "When true, enable DDB PITR and deletion protection; S3 objects are not force-destroyed."
}

variable "log_retention_in_days" {
  type        = number
  default     = 7
  description = "CloudWatch Logs retention for shared observability log groups (e.g. EMF metrics)."
}
