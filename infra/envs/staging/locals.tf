locals {
  run_env                 = "staging"
  shared_config           = yamldecode(file("${path.module}/../../../config/shared/${local.run_env}.yaml"))
  artifacts_bucket_name   = local.shared_config.artifacts.bucket
  catalog_table_name      = local.shared_config.catalog.table
  publish_lock_table_name = local.shared_config.catalog.publish_lock_table

  eif_source_path = var.eif_source_path != "" ? var.eif_source_path : abspath("${path.module}/../../../.nitrum/artifacts/${var.project_name}.eif")
}
