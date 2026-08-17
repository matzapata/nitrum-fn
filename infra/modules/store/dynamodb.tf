resource "aws_dynamodb_table" "catalog" {
  name                        = local.catalog_table_name
  billing_mode                = "PAY_PER_REQUEST"
  deletion_protection_enabled = var.retain

  hash_key  = "fn_id"
  range_key = "label"

  attribute {
    name = "fn_id"
    type = "S"
  }

  attribute {
    name = "label"
    type = "S"
  }

  server_side_encryption {
    enabled = true
  }

  point_in_time_recovery {
    enabled = var.retain
  }
}
