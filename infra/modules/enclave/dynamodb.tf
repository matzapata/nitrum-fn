resource "aws_dynamodb_table" "enclave" {
  name                        = var.project_name
  billing_mode                = "PAY_PER_REQUEST"
  deletion_protection_enabled = var.retain

  hash_key = "pk"

  attribute {
    name = "pk"
    type = "S"
  }

  ttl {
    attribute_name = "ttl"
    enabled        = true
  }

  server_side_encryption {
    enabled = true
  }

  point_in_time_recovery {
    enabled = var.retain
  }
}
