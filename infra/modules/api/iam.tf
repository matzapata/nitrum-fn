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
  name_prefix           = "${var.project_name}-api-exec-"
  assume_role_policy    = data.aws_iam_policy_document.task_assume.json
  force_detach_policies = true
}

resource "aws_iam_role_policy_attachment" "execution" {
  role       = aws_iam_role.execution.name
  policy_arn = "arn:aws:iam::aws:policy/service-role/AmazonECSTaskExecutionRolePolicy"
}

resource "aws_iam_role" "task" {
  name_prefix           = "${var.project_name}-api-task-"
  assume_role_policy    = data.aws_iam_policy_document.task_assume.json
  force_detach_policies = true
}

data "aws_iam_policy_document" "task" {
  statement {
    sid    = "ArtifactsReadWrite"
    effect = "Allow"
    actions = [
      "s3:GetObject",
      "s3:PutObject",
      "s3:AbortMultipartUpload",
    ]
    resources = ["${var.artifacts_bucket_arn}/artifacts/*"]
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
    sid    = "CatalogRead"
    effect = "Allow"
    actions = [
      "dynamodb:GetItem",
      "dynamodb:Query",
    ]
    resources = [var.catalog_table_arn]
  }

  statement {
    sid       = "PublishSns"
    effect    = "Allow"
    actions   = ["sns:Publish"]
    resources = [var.publish_topic_arn]
  }
}

resource "aws_iam_role_policy" "task" {
  name   = "NitrumFnApiStore"
  role   = aws_iam_role.task.id
  policy = data.aws_iam_policy_document.task.json
}
