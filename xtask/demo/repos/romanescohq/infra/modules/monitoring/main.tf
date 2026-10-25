# Alarms for the platform, and where they page.

resource "aws_sns_topic" "oncall" {
  name = "${var.environment}-oncall"
}

resource "aws_sns_topic_subscription" "pager" {
  count     = var.pager_endpoint == null ? 0 : 1
  topic_arn = aws_sns_topic.oncall.arn
  protocol  = "https"
  endpoint  = var.pager_endpoint
}

resource "aws_cloudwatch_metric_alarm" "api_5xx" {
  alarm_name          = "${var.environment}-api-5xx"
  alarm_description   = "The platform API answers more than ${var.api_5xx_per_minute} requests a minute with a 5xx. Runbook: docs/runbooks/api-errors.md"
  namespace           = "AWS/ApplicationELB"
  metric_name         = "HTTPCode_Target_5XX_Count"
  dimensions          = { LoadBalancer = var.load_balancer }
  statistic           = "Sum"
  period              = 60
  evaluation_periods  = 5
  threshold           = var.api_5xx_per_minute
  comparison_operator = "GreaterThanThreshold"
  treat_missing_data  = "notBreaching"
  alarm_actions       = [aws_sns_topic.oncall.arn]
  ok_actions          = [aws_sns_topic.oncall.arn]
}

resource "aws_cloudwatch_metric_alarm" "database_cpu" {
  alarm_name          = "${var.environment}-database-cpu"
  alarm_description   = "The database's CPU has been above 80% for 15 minutes. Runbook: docs/runbooks/database-failover.md"
  namespace           = "AWS/RDS"
  metric_name         = "CPUUtilization"
  dimensions          = { DBInstanceIdentifier = var.database_id }
  statistic           = "Average"
  period              = 300
  evaluation_periods  = 3
  threshold           = 80
  comparison_operator = "GreaterThanThreshold"
  alarm_actions       = [aws_sns_topic.oncall.arn]
  ok_actions          = [aws_sns_topic.oncall.arn]
}
