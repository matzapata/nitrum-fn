data "aws_caller_identity" "current" {}

locals {
  eif_bucket_name       = "${var.project_name}-eif-${data.aws_caller_identity.current.account_id}"
  artifacts_bucket_name = "${var.project_name}-artifacts-${data.aws_caller_identity.current.account_id}"
  catalog_table_name    = "${var.project_name}-fn-catalog"
}
