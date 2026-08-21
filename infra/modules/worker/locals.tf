data "aws_region" "current" {}

locals {
  container_name = "nitrum-fn-publish-worker"
  image          = "${aws_ecr_repository.worker.repository_url}:${var.image_tag}"
}
