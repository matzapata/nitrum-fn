data "aws_iam_policy_document" "task_assume" {
  statement {
    effect = "Allow"
    principals {
      type        = "Service"
      identifiers = ["ecs-tasks.amazonaws.com"]
    }
    actions = ["sts:AssumeRole"]
  }
}

resource "aws_iam_role" "execution" {
  name_prefix           = "${var.project_name}-worker-exec-"
  assume_role_policy    = data.aws_iam_policy_document.task_assume.json
  force_detach_policies = true
}

resource "aws_iam_role_policy_attachment" "execution" {
  role       = aws_iam_role.execution.name
  policy_arn = "arn:aws:iam::aws:policy/service-role/AmazonECSTaskExecutionRolePolicy"
}

resource "aws_iam_role" "task" {
  name_prefix           = "${var.project_name}-worker-task-"
  assume_role_policy    = data.aws_iam_policy_document.task_assume.json
  force_detach_policies = true
}

data "aws_iam_policy_document" "task" {
  statement {
    sid    = "ArtifactsRead"
    effect = "Allow"
    actions = [
      "s3:GetObject",
    ]
    resources = ["${var.artifacts_bucket_arn}/artifacts/*"]
  }

  statement {
    sid    = "ArtifactsCompiledWrite"
    effect = "Allow"
    actions = [
      "s3:PutObject",
      "s3:AbortMultipartUpload",
    ]
    resources = ["${var.artifacts_bucket_arn}/artifacts/*.cwasm"]
  }

  statement {
    sid       = "ArtifactsList"
    effect    = "Allow"
    actions   = ["s3:ListBucket"]
    resources = [var.artifacts_bucket_arn]
    condition {
      test     = "StringLike"
      variable = "s3:prefix"
      values   = ["artifacts", "artifacts/*"]
    }
  }

  statement {
    sid    = "CatalogReadWrite"
    effect = "Allow"
    actions = [
      "dynamodb:GetItem",
      "dynamodb:PutItem",
      "dynamodb:UpdateItem",
      "dynamodb:DeleteItem",
      "dynamodb:Query",
    ]
    resources = [var.catalog_table_arn]
  }

  statement {
    sid    = "CompileQueue"
    effect = "Allow"
    actions = [
      "sqs:ReceiveMessage",
      "sqs:DeleteMessage",
      "sqs:GetQueueAttributes",
      "sqs:ChangeMessageVisibility",
    ]
    resources = [var.compile_queue_arn]
  }
}

resource "aws_iam_role_policy" "task" {
  name   = "NitrumFnWorker"
  role   = aws_iam_role.task.id
  policy = data.aws_iam_policy_document.task.json
}
