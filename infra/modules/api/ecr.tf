resource "aws_ecr_repository" "api" {
  name                 = "${var.project_name}/nitrum-fn-api"
  image_tag_mutability = "MUTABLE"

  image_scanning_configuration {
    scan_on_push = true
  }
}

resource "aws_cloudwatch_log_group" "api" {
  name              = "/nitrum/${var.project_name}/api"
  retention_in_days = var.log_retention_in_days
}
