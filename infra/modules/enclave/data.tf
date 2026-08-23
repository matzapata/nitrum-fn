data "aws_caller_identity" "current" {}

data "aws_region" "current" {}

data "aws_ssm_parameter" "al2023" {
  count = var.ami_id == "" ? 1 : 0
  name  = "/aws/service/ami-amazon-linux-latest/al2023-ami-kernel-default-x86_64"
}

locals {
  # Control-plane downloads s3://bucket/{eif-hash}.eif (--eif-hash is eif_version_label).
  eif_object_key = "${var.eif_version_label}.eif"

  ami_id = var.ami_id != "" ? var.ami_id : data.aws_ssm_parameter.al2023[0].value

  kms_admin_arn = var.kms_administrator_role_arn == "AWS_ACCOUNT_ROOT" ? "arn:aws:iam::${data.aws_caller_identity.current.account_id}:root" : var.kms_administrator_role_arn

  instance_refresh_min_healthy = var.rolling_min_instances_in_service > 0 ? 100 : 0
  instance_refresh_max_healthy = var.rolling_min_instances_in_service > 0 ? 200 : 100
}
