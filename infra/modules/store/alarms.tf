resource "aws_cloudwatch_metric_alarm" "compile_dlq_depth" {
  count = var.sns_alarm_topic_arn != "" ? 1 : 0

  alarm_name          = "${var.project_name}-fn-compile-dlq-depth"
  alarm_description   = "Compile DLQ has messages — poison or repeatedly failing compile jobs (${var.project_name})"
  namespace           = "AWS/SQS"
  metric_name         = "ApproximateNumberOfMessagesVisible"
  statistic           = "Maximum"
  period              = 60
  evaluation_periods  = 1
  threshold           = 1
  comparison_operator = "GreaterThanOrEqualToThreshold"
  treat_missing_data  = "notBreaching"

  dimensions = {
    QueueName = aws_sqs_queue.compile_dlq.name
  }

  alarm_actions = [var.sns_alarm_topic_arn]
  ok_actions    = [var.sns_alarm_topic_arn]
}
