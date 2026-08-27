variable "project_name" {
  type        = string
  description = "Project slug used in bucket/table names and SSM paths."

  validation {
    condition     = can(regex("^[a-z][a-z0-9-]{2,127}$", var.project_name))
    error_message = "project_name must match ^[a-z][a-z0-9-]{2,127}$."
  }
}

variable "retain" {
  type        = bool
  default     = false
  description = "When true, enable DDB PITR and deletion protection; S3 objects are not force-destroyed."
}

variable "eif_s3_key" {
  type        = string
  default     = "enclave.eif"
  description = "S3 object key of the EIF (control-plane expects {eif-hash}.eif)."
}

variable "sns_alarm_topic_arn" {
  type        = string
  default     = ""
  description = "Optional SNS topic ARN for CloudWatch alarms. Empty string disables alarm resources."
}

variable "log_retention_in_days" {
  type        = number
  default     = 7
  description = "CloudWatch Logs retention for shared observability log groups (e.g. EMF metrics)."
}
