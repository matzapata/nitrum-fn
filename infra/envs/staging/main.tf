module "network" {
  source = "../../modules/network"

  project_name = var.project_name
}

module "store" {
  source = "../../modules/store"

  project_name          = var.project_name
  retain                = var.retain
  eif_s3_key            = local.eif_s3_key
  sns_alarm_topic_arn   = var.sns_alarm_topic_arn
}

module "api" {
  source = "../../modules/api"

  project_name           = var.project_name
  vpc_id                 = module.network.vpc_id
  public_subnet_ids      = module.network.public_subnet_ids
  private_subnet_ids     = module.network.private_subnet_ids
  artifacts_bucket_name  = module.store.artifacts_bucket_name
  artifacts_bucket_arn   = module.store.artifacts_bucket_arn
  catalog_table_name       = module.store.catalog_table_name
  catalog_table_arn        = module.store.catalog_table_arn
  publish_lock_table_name   = module.store.publish_lock_table_name
  publish_lock_table_arn    = module.store.publish_lock_table_arn
  publish_topic_arn        = module.store.publish_topic_arn
  image                  = var.api_image
  desired_count          = var.api_desired_count
  log_retention_in_days  = var.log_retention_in_days
}

module "worker" {
  source = "../../modules/worker"

  project_name          = var.project_name
  vpc_id                = module.network.vpc_id
  private_subnet_ids    = module.network.private_subnet_ids
  artifacts_bucket_name = module.store.artifacts_bucket_name
  artifacts_bucket_arn  = module.store.artifacts_bucket_arn
  catalog_table_name     = module.store.catalog_table_name
  catalog_table_arn      = module.store.catalog_table_arn
  publish_lock_table_name = module.store.publish_lock_table_name
  publish_lock_table_arn  = module.store.publish_lock_table_arn
  compile_queue_url      = module.store.compile_queue_url
  compile_queue_arn      = module.store.compile_queue_arn
  image                 = var.worker_image
  desired_count         = var.worker_desired_count
  log_retention_in_days = var.log_retention_in_days
}

module "enclave" {
  count  = var.enable_enclave ? 1 : 0
  source = "../../modules/enclave"

  project_name                     = var.project_name
  retain                           = var.retain
  vpc_id                           = module.network.vpc_id
  vpc_cidr_block                   = module.network.vpc_cidr_block
  public_subnet_ids                = module.network.public_subnet_ids
  private_subnet_ids               = module.network.private_subnet_ids
  eif_s3_bucket                    = module.store.eif_bucket_name
  eif_s3_key                       = module.store.eif_s3_key
  eif_source_path                  = local.eif_source_path
  eif_version_label                = var.eif_version_label
  eif_image_sha384                 = var.eif_image_sha384
  asg_min_size                     = var.asg_min_size
  asg_max_size                     = var.asg_max_size
  asg_desired_capacity             = var.asg_desired_capacity
  enclave_cpu_count                = var.enclave_cpu_count
  enclave_memory_mib               = var.enclave_memory_mib
  instance_type                    = var.instance_type
  rolling_min_instances_in_service = var.rolling_min_instances_in_service
  enable_xray_tracing              = var.enable_xray_tracing
  log_retention_in_days            = var.log_retention_in_days
  sns_alarm_topic_arn              = var.sns_alarm_topic_arn
  control_plane_image              = var.control_plane_image
  control_plane_debug_arg          = var.control_plane_debug_arg
  otel_collector_image             = var.otel_collector_image
  kms_administrator_role_arn       = var.kms_administrator_role_arn
}

data "aws_iam_policy_document" "enclave_fn_store" {
  count = var.enable_enclave ? 1 : 0

  statement {
    sid       = "ArtifactsRead"
    effect    = "Allow"
    actions   = ["s3:GetObject"]
    resources = ["${module.store.artifacts_bucket_arn}/artifacts/*"]
  }

  statement {
    sid    = "CatalogRead"
    effect = "Allow"
    actions = [
      "dynamodb:GetItem",
      "dynamodb:Query",
    ]
    resources = [module.store.catalog_table_arn]
  }
}

resource "aws_iam_role_policy" "enclave_fn_store" {
  count  = var.enable_enclave ? 1 : 0
  name   = "NitrumFnStoreRead"
  role   = module.enclave[0].instance_role_name
  policy = data.aws_iam_policy_document.enclave_fn_store[0].json
}

resource "aws_route53_record" "invoke" {
  count   = var.enable_enclave && var.hosted_zone_id != "" && var.invoke_hostname != "" ? 1 : 0
  zone_id = var.hosted_zone_id
  name    = var.invoke_hostname
  type    = "A"

  alias {
    name                   = module.enclave[0].nlb_dns_name
    zone_id                = module.enclave[0].nlb_zone_id
    evaluate_target_health = true
  }
}

check "enclave_pcr0" {
  assert {
    condition     = !var.enable_enclave || var.eif_image_sha384 != ""
    error_message = "eif_image_sha384 is required when enable_enclave is true (PCR0 from nitro-cli describe-eif)."
  }
}

check "eif_source" {
  assert {
    condition     = !var.enable_enclave || fileexists(local.eif_source_path)
    error_message = "enable_enclave requires a local EIF. Run `nitrum build` (expected at .nitrum/artifacts/${var.project_name}.eif)."
  }
}

check "invoke_dns" {
  assert {
    condition     = var.invoke_hostname == "" || var.hosted_zone_id != ""
    error_message = "hosted_zone_id is required when invoke_hostname is set."
  }
}
