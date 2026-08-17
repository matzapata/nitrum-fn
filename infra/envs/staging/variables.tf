variable "aws_region" {
  type        = string
  default     = "us-east-1"
  description = "AWS region for this environment."
}

variable "project_name" {
  type        = string
  description = "Must match nitrum.toml [project].name. SSM, KMS alias, and Nitrum data-plane paths use this slug."

  validation {
    condition     = can(regex("^[a-z][a-z0-9-]{2,127}$", var.project_name))
    error_message = "project_name must match ^[a-z][a-z0-9-]{2,127}$."
  }
}

variable "retain" {
  type        = bool
  default     = false
  description = "When true, enable rotation / PITR / DDB deletion protection (typical prod). Staging default is false."
}

variable "eif_s3_key" {
  type        = string
  default     = "enclave.eif"
  description = "S3 object key of the EIF file in the EIF bucket."
}

variable "enable_enclave" {
  type        = bool
  default     = false
  description = "When false, deploy network + store + API only (no Nitro ASG/NLB/KMS). Set true after you have an EIF and PCR0."
}

variable "eif_version_label" {
  type        = string
  default     = "000000000000"
  description = "Short label for this EIF build (e.g. first 12 hex chars of sha256). Bump on every EIF upload. Required when enable_enclave is true."
}

variable "eif_image_sha384" {
  type        = string
  default     = ""
  description = "PCR0 / enclave image SHA-384 (hex) from nitro-cli describe-eif. Required when enable_enclave is true."
}

variable "hosted_zone_id" {
  type        = string
  description = "Route53 hosted zone ID for the API hostname and ACM validation."
}

variable "api_hostname" {
  type        = string
  description = "FQDN for the management API, e.g. api.staging.example.com."
}

variable "invoke_hostname" {
  type        = string
  default     = ""
  description = "Optional FQDN for invoke (NLB alias), e.g. fn.staging.example.com. Empty skips the record. Only used when enable_enclave is true."
}

variable "api_image_tag" {
  type        = string
  default     = "latest"
  description = "ECR image tag for nitrum-fn-api."
}

variable "api_desired_count" {
  type        = number
  default     = 1
  description = "Fargate desired count. Push an image to ECR before the service can start."
}

variable "asg_min_size" {
  type    = number
  default = 1
}

variable "asg_max_size" {
  type    = number
  default = 2
}

variable "asg_desired_capacity" {
  type    = number
  default = 1
}

variable "enclave_cpu_count" {
  type    = number
  default = 2
}

variable "enclave_memory_mib" {
  type    = number
  default = 4320
}

variable "instance_type" {
  type    = string
  default = "m6i.xlarge"
}

variable "rolling_min_instances_in_service" {
  type    = number
  default = 1
}

variable "enable_xray_tracing" {
  type    = bool
  default = false
}

variable "log_retention_in_days" {
  type    = number
  default = 7
}

variable "sns_alarm_topic_arn" {
  type    = string
  default = ""
}

variable "control_plane_image" {
  type    = string
  default = "ghcr.io/matzapata/nitrum/control-plane:latest"
}

variable "control_plane_debug_arg" {
  type    = string
  default = ""
}

variable "otel_collector_image" {
  type    = string
  default = "public.ecr.aws/aws-observability/aws-otel-collector:latest"
}

variable "kms_administrator_role_arn" {
  type    = string
  default = "AWS_ACCOUNT_ROOT"
}
