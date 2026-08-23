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
  default     = ""
  description = "S3 object key of the EIF. Empty (recommended) uses {eif_version_label}.eif, which is what the Nitrum control-plane downloads."
}

variable "eif_source_path" {
  type        = string
  default     = ""
  description = "Local EIF path for Terraform to upload when enable_enclave is true. Empty uses <repo>/.nitrum/artifacts/<project_name>.eif."
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
  default     = ""
  description = "Optional Route53 hosted zone. Only needed if invoke_hostname is set. The API uses the ALB DNS name (HTTP)."
}

variable "invoke_hostname" {
  type        = string
  default     = ""
  description = "Optional FQDN for invoke (NLB alias). Empty skips the record. Requires hosted_zone_id. Default invoke URL is the NLB DNS name."
}

variable "api_image" {
  type        = string
  default     = "ghcr.io/matzapata/nitrum-fn/api:latest"
  description = "Full container image URI for nitrum-fn-api. Override for Docker Hub or another GHCR repo."
}

variable "api_desired_count" {
  type        = number
  default     = 1
  description = "Fargate desired count. The image must exist in the registry before the service can start."
}

variable "worker_image" {
  type        = string
  default     = "ghcr.io/matzapata/nitrum-fn/publish-worker:latest"
  description = "Full container image URI for nitrum-fn-publish-worker. Override for Docker Hub or another GHCR repo."
}

variable "worker_desired_count" {
  type        = number
  default     = 1
  description = "Publish worker Fargate desired count."
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
