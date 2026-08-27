resource "aws_cloudwatch_log_group" "data_plane" {
  name              = "/nitrum/${var.project_name}/data-plane"
  retention_in_days = var.log_retention_in_days
}

resource "aws_cloudwatch_log_group" "control_plane" {
  name              = "/nitrum/${var.project_name}/control-plane"
  retention_in_days = var.log_retention_in_days
}
