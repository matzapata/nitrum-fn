data "aws_caller_identity" "current" {}

data "aws_region" "current" {}

locals {
  container_name = "nitrum-fn-api"
  container_port = 8080
  image          = "${aws_ecr_repository.api.repository_url}:${var.image_tag}"
}
