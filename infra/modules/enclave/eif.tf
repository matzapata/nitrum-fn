# Uploads the local nitrum-build EIF so the control-plane can download it at boot.
# EIFs are large: use source_hash, not etag (S3 multipart ETags do not match filemd5).
resource "aws_s3_object" "eif" {
  bucket      = var.eif_s3_bucket
  key         = local.eif_object_key
  source      = var.eif_source_path
  source_hash = fileexists(var.eif_source_path) ? filemd5(var.eif_source_path) : null

  lifecycle {
    precondition {
      condition     = fileexists(var.eif_source_path)
      error_message = "Local EIF missing at ${var.eif_source_path}. Run `nitrum build` from the repo root before apply with enable_enclave = true."
    }
  }
}
