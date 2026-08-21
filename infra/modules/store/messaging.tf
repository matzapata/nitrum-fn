resource "aws_sns_topic" "publish" {
  name = "${var.project_name}-fn-publish"
}

resource "aws_sqs_queue" "compile_dlq" {
  name                      = "${var.project_name}-fn-compile-dlq"
  message_retention_seconds = 1209600
}

resource "aws_sqs_queue" "compile" {
  name                       = "${var.project_name}-fn-compile"
  visibility_timeout_seconds = 300
  receive_wait_time_seconds  = 20
  redrive_policy = jsonencode({
    deadLetterTargetArn = aws_sqs_queue.compile_dlq.arn
    maxReceiveCount     = 3
  })
}

resource "aws_sns_topic_subscription" "compile" {
  topic_arn            = aws_sns_topic.publish.arn
  protocol             = "sqs"
  endpoint             = aws_sqs_queue.compile.arn
  raw_message_delivery = true
}

data "aws_iam_policy_document" "compile_queue" {
  statement {
    sid    = "AllowSnsPublish"
    effect = "Allow"
    principals {
      type        = "Service"
      identifiers = ["sns.amazonaws.com"]
    }
    actions   = ["sqs:SendMessage"]
    resources = [aws_sqs_queue.compile.arn]
    condition {
      test     = "ArnEquals"
      variable = "aws:SourceArn"
      values   = [aws_sns_topic.publish.arn]
    }
    condition {
      test     = "StringEquals"
      variable = "aws:SourceAccount"
      values   = [data.aws_caller_identity.current.account_id]
    }
  }
}

resource "aws_sqs_queue_policy" "compile" {
  queue_url = aws_sqs_queue.compile.id
  policy    = data.aws_iam_policy_document.compile_queue.json
}
