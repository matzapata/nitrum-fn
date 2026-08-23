data "aws_region" "current" {}

locals {
  container_name = "nitrum-fn-publish-worker"
}
