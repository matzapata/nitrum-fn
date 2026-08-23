resource "aws_cloudwatch_log_group" "api" {
  name              = "/nitrum/${var.project_name}/api"
  retention_in_days = var.log_retention_in_days
}
