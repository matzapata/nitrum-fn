data "aws_iam_policy_document" "kms" {
  statement {
    sid    = "EnableDecryptFromEnclave"
    effect = "Allow"
    principals {
      type        = "AWS"
      identifiers = ["arn:aws:iam::${data.aws_caller_identity.current.account_id}:root"]
    }
    actions   = ["kms:Decrypt"]
    resources = ["*"]
    condition {
      test     = "StringEqualsIgnoreCase"
      variable = "kms:RecipientAttestation:ImageSha384"
      values   = [var.eif_image_sha384]
    }
  }

  statement {
    sid    = "EnclaveGenerateDataKey"
    effect = "Allow"
    principals {
      type        = "AWS"
      identifiers = ["arn:aws:iam::${data.aws_caller_identity.current.account_id}:root"]
    }
    actions = [
      "kms:GenerateDataKey",
      "kms:GenerateDataKeyWithoutPlaintext",
    ]
    resources = ["*"]
  }

  statement {
    sid    = "KmsAdministrator"
    effect = "Allow"
    principals {
      type        = "AWS"
      identifiers = [local.kms_admin_arn]
    }
    actions = [
      "kms:Create*",
      "kms:Describe*",
      "kms:Enable*",
      "kms:List*",
      "kms:Put*",
      "kms:Update*",
      "kms:Revoke*",
      "kms:Disable*",
      "kms:Get*",
      "kms:Delete*",
      "kms:ScheduleKeyDeletion",
      "kms:CancelKeyDeletion",
      "kms:GenerateDataKey",
      "kms:TagResource",
      "kms:UntagResource",
    ]
    resources = ["*"]
  }
}

resource "aws_kms_key" "enclave" {
  description             = "${var.project_name} | enclave CMK (GenerateDataKey + attested Decrypt)"
  enable_key_rotation     = var.retain
  deletion_window_in_days = var.retain ? 30 : 7
  policy                  = data.aws_iam_policy_document.kms.json
}

resource "aws_kms_alias" "enclave" {
  name          = "alias/${var.project_name}-enclave"
  target_key_id = aws_kms_key.enclave.key_id
}
