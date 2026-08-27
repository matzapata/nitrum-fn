resource "aws_cloudwatch_log_group" "metrics" {
  name              = "/nitrum/${var.project_name}/metrics"
  retention_in_days = var.log_retention_in_days
}
