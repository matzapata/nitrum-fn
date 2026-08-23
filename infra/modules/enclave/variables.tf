variable "project_name" {
  type        = string
  description = "Project slug: DynamoDB table, KMS alias, log groups, tags, launch template, SSM paths."

  validation {
    condition     = can(regex("^[a-z][a-z0-9-]{2,127}$", var.project_name))
    error_message = "project_name must match ^[a-z][a-z0-9-]{2,127}$."
  }
}

variable "vpc_id" {
  type        = string
  description = "VPC from the network module."
}

variable "vpc_cidr_block" {
  type        = string
  description = "VPC CIDR (instance SG rules)."
}

variable "public_subnet_ids" {
  type        = list(string)
  description = "Public subnet IDs for the NLB."
}

variable "private_subnet_ids" {
  type        = list(string)
  description = "Private subnet IDs for the enclave ASG."
}

variable "retain" {
  type        = bool
  default     = false
  description = "When true, enable KMS rotation, DDB PITR and deletion protection, and keep EBS on terminate (typical prod)."
}

variable "eif_s3_bucket" {
  type        = string
  description = "S3 bucket where the enclave EIF object is uploaded."
}

variable "eif_s3_key" {
  type        = string
  default     = ""
  description = "Unused for the object key (always {eif_version_label}.eif). Kept so callers can still pass a value."
}

variable "eif_source_path" {
  type        = string
  description = "Absolute or module-relative path to the local EIF from `nitrum build` (uploaded as {eif_version_label}.eif)."
}

variable "eif_version_label" {
  type        = string
  default     = "000000000000"
  description = "Short label for this EIF build (e.g. first 12 hex chars of sha256). Change on every new upload so the launch template name changes and the ASG can roll instances."
}

variable "eif_image_sha384" {
  type        = string
  description = "PCR0 / enclave image SHA-384 (hex) used in the KMS key policy with kms:RecipientAttestation:ImageSha384."

  validation {
    condition     = length(var.eif_image_sha384) >= 64
    error_message = "eif_image_sha384 must be the PCR0 hex from nitro-cli describe-eif."
  }
}

variable "asg_min_size" {
  type        = number
  default     = 1
  description = "Auto Scaling group minimum size."
}

variable "asg_max_size" {
  type        = number
  default     = 2
  description = "Auto Scaling group maximum size."
}

variable "asg_desired_capacity" {
  type        = number
  default     = 1
  description = "Auto Scaling group desired capacity."
}

variable "enclave_cpu_count" {
  type        = number
  default     = 2
  description = "vCPUs per Nitro Enclave (nitro-cli --cpu-count)."
}

variable "enclave_memory_mib" {
  type        = number
  default     = 4320
  description = "Enclave memory in MiB (nitro-cli --memory)."
}

variable "instance_type" {
  type        = string
  default     = "m6i.xlarge"
  description = "EC2 instance type for Nitro Enclave hosts."
}

variable "rolling_min_instances_in_service" {
  type        = number
  default     = 1
  description = "When > 0, instance refresh keeps capacity (zero-downtime). 0 replaces in place."
}

variable "enable_xray_tracing" {
  type        = bool
  default     = false
  description = "When true, ADOT exports OTLP traces to X-Ray."
}

variable "log_retention_in_days" {
  type        = number
  default     = 7
  description = "CloudWatch Logs retention for data-plane, control-plane, and metrics log groups."
}

variable "sns_alarm_topic_arn" {
  type        = string
  default     = ""
  description = "Optional SNS topic ARN for CloudWatch alarms. Empty string disables alarm resources."
}

variable "ami_id" {
  type        = string
  default     = ""
  description = "Optional AMI id. Empty uses the latest Amazon Linux 2023 x86_64 AMI from SSM."
}

variable "control_plane_image" {
  type        = string
  default     = "ghcr.io/matzapata/nitrum/control-plane:latest"
  description = "Docker image for the Nitrum control-plane on EC2."
}

variable "control_plane_debug_arg" {
  type        = string
  default     = ""
  description = "Optional control-plane debug CLI arg (`--debug-mode` or empty)."
}

variable "otel_collector_image" {
  type        = string
  default     = "public.ecr.aws/aws-observability/aws-otel-collector:latest"
  description = "ADOT Collector image run on the EC2 host."
}

variable "kms_administrator_role_arn" {
  type        = string
  default     = "AWS_ACCOUNT_ROOT"
  description = "Principal allowed to administer this KMS key. AWS_ACCOUNT_ROOT uses the account root ARN."
}

variable "instance_managed_policy_arns" {
  type        = list(string)
  default     = []
  description = "Optional extra IAM managed policy ARNs attached to the EC2 instance role (in addition to AmazonSSMManagedInstanceCore)."
}
