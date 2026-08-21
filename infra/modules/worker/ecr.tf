resource "aws_ecr_repository" "worker" {
  name                 = "${var.project_name}/nitrum-fn-publish-worker"
  image_tag_mutability = "MUTABLE"

  image_scanning_configuration {
    scan_on_push = true
  }
}

resource "aws_cloudwatch_log_group" "worker" {
  name              = "/nitrum/${var.project_name}/publish-worker"
  retention_in_days = var.log_retention_in_days
}
