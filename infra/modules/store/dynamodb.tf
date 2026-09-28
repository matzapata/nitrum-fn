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

resource "aws_dynamodb_table" "publish_lock" {
  name                        = local.publish_lock_table_name
  billing_mode                = "PAY_PER_REQUEST"
  deletion_protection_enabled = var.retain

  hash_key = "fn_id"

  attribute {
    name = "fn_id"
    type = "S"
  }

  ttl {
    attribute_name = "expires_at"
    enabled        = true
  }

  server_side_encryption {
    enabled = true
  }

  point_in_time_recovery {
    enabled = var.retain
  }
}

resource "aws_dynamodb_table" "accounts" {
  name                        = local.accounts_table_name
  billing_mode                = "PAY_PER_REQUEST"
  deletion_protection_enabled = var.retain

  hash_key = "account_id"

  attribute {
    name = "account_id"
    type = "S"
  }

  server_side_encryption {
    enabled = true
  }

  point_in_time_recovery {
    enabled = var.retain
  }
}

resource "aws_dynamodb_table" "keys" {
  name                        = local.keys_table_name
  billing_mode                = "PAY_PER_REQUEST"
  deletion_protection_enabled = var.retain

  hash_key = "secret_hash"

  attribute {
    name = "secret_hash"
    type = "S"
  }

  attribute {
    name = "account_id"
    type = "S"
  }

  global_secondary_index {
    name            = "account_id_index"
    hash_key        = "account_id"
    projection_type = "ALL"
  }

  server_side_encryption {
    enabled = true
  }

  point_in_time_recovery {
    enabled = var.retain
  }
}

resource "aws_dynamodb_table" "credit_receipts" {
  name                        = local.receipts_table_name
  billing_mode                = "PAY_PER_REQUEST"
  deletion_protection_enabled = var.retain

  hash_key = "nonce"

  attribute {
    name = "nonce"
    type = "S"
  }

  server_side_encryption {
    enabled = true
  }

  point_in_time_recovery {
    enabled = var.retain
  }
}
