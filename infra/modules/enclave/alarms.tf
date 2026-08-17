resource "aws_cloudwatch_metric_alarm" "unhealthy_hosts" {
  count = var.sns_alarm_topic_arn != "" ? 1 : 0

  alarm_name          = "${var.project_name}-nlb-unhealthy-hosts"
  alarm_description   = "NLB HTTPS target group has unhealthy hosts (${var.project_name})"
  namespace           = "AWS/NetworkELB"
  metric_name         = "UnHealthyHostCount"
  statistic           = "Maximum"
  period              = 60
  evaluation_periods  = 2
  threshold           = 1
  comparison_operator = "GreaterThanOrEqualToThreshold"
  treat_missing_data  = "notBreaching"

  dimensions = {
    TargetGroup  = aws_lb_target_group.https.arn_suffix
    LoadBalancer = aws_lb.nlb.arn_suffix
  }

  alarm_actions = [var.sns_alarm_topic_arn]
  ok_actions    = [var.sns_alarm_topic_arn]
}

resource "aws_cloudwatch_metric_alarm" "asg_capacity" {
  count = var.sns_alarm_topic_arn != "" ? 1 : 0

  alarm_name          = "${var.project_name}-asg-low-capacity"
  alarm_description   = "ASG in-service instances below minimum (${var.project_name})"
  namespace           = "AWS/AutoScaling"
  metric_name         = "GroupInServiceInstances"
  statistic           = "Average"
  period              = 60
  evaluation_periods  = 3
  threshold           = var.asg_min_size
  comparison_operator = "LessThanThreshold"
  treat_missing_data  = "breaching"

  dimensions = {
    AutoScalingGroupName = aws_autoscaling_group.nitro.name
  }

  alarm_actions = [var.sns_alarm_topic_arn]
  ok_actions    = [var.sns_alarm_topic_arn]
}
