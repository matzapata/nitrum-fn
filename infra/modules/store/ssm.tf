data "aws_region" "current" {}

# data-plane clears the user process env and reinjects only /nitrum/.../env/*.
resource "aws_ssm_parameter" "aws_region" {
  name        = "/nitrum/${var.project_name}/env/AWS_REGION"
  type        = "String"
  value       = data.aws_region.current.name
  description = "AWS region for the enclave host SDK (user process env is cleared except SSM + OTel)"
}

resource "aws_ssm_parameter" "run_env" {
  name        = "/nitrum/${var.project_name}/env/NITRUM_FN_ENV"
  type        = "String"
  value       = var.run_env
  description = "Config overlay for the enclave host (selects config/shared/{env}.yaml artifacts.bucket)"
}
