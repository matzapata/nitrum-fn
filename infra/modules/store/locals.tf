data "aws_caller_identity" "current" {}

locals {
  eif_bucket_name = "${var.project_name}-eif-${data.aws_caller_identity.current.account_id}"
  # Bucket/table names come from config/shared/{run_env}.yaml via variables.
  artifacts_bucket_name   = var.artifacts_bucket_name
  catalog_table_name      = var.catalog_table_name
  publish_lock_table_name = var.publish_lock_table_name
}
