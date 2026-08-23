data "aws_region" "current" {}

locals {
  container_name = "nitrum-fn-api"
  container_port = 8080
}
