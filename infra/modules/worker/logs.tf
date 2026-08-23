resource "aws_cloudwatch_log_group" "worker" {
  name              = "/nitrum/${var.project_name}/publish-worker"
  retention_in_days = var.log_retention_in_days
}
