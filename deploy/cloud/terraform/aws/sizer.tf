# The sizer: every minute, the pool's demand from fctl, into CloudWatch and
# into the group's desired capacity (sizer/index.py).

data "archive_file" "sizer" {
  type        = "zip"
  source_file = "${path.module}/sizer/index.py"
  output_path = "${path.root}/.terraform/fctl-sizer.zip"
}

resource "aws_iam_role" "sizer" {
  name = "${var.name}-sizer"
  tags = var.tags
  assume_role_policy = jsonencode({
    Version   = "2012-10-17"
    Statement = [{ Effect = "Allow", Principal = { Service = "lambda.amazonaws.com" }, Action = "sts:AssumeRole" }]
  })
}

resource "aws_iam_role_policy" "sizer" {
  name = "${var.name}-sizer"
  role = aws_iam_role.sizer.id
  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      { Effect = "Allow", Action = "secretsmanager:GetSecretValue", Resource = aws_secretsmanager_secret.token.arn },
      { Effect = "Allow", Action = "autoscaling:SetDesiredCapacity", Resource = local.group_arn },
      # Neither takes a resource narrower than all.
      { Effect = "Allow", Action = ["autoscaling:DescribeAutoScalingGroups", "cloudwatch:PutMetricData"], Resource = "*" },
      {
        Effect   = "Allow"
        Action   = ["logs:CreateLogStream", "logs:PutLogEvents"]
        Resource = "${aws_cloudwatch_log_group.sizer.arn}:*"
      },
    ]
  })
}

resource "aws_cloudwatch_log_group" "sizer" {
  name              = "/aws/lambda/${var.name}-sizer"
  retention_in_days = 14
  tags              = var.tags
}

resource "aws_lambda_function" "sizer" {
  function_name    = "${var.name}-sizer"
  role             = aws_iam_role.sizer.arn
  runtime          = "python3.13"
  handler          = "index.handler"
  filename         = data.archive_file.sizer.output_path
  source_code_hash = data.archive_file.sizer.output_base64sha256
  timeout          = 30
  tags             = var.tags

  environment {
    variables = {
      FCTL_SERVER = var.fctl_server
      SECRET      = aws_secretsmanager_secret.token.arn
      GROUP       = aws_autoscaling_group.runners.name
      RAISE_ONLY  = local.retire ? "1" : "0"
    }
  }

  depends_on = [aws_cloudwatch_log_group.sizer]
}

resource "aws_cloudwatch_event_rule" "sizer" {
  name                = "${var.name}-sizer"
  description         = "Sizes the fctl runner pool's group every minute"
  schedule_expression = "rate(1 minute)"
  tags                = var.tags
}

resource "aws_cloudwatch_event_target" "sizer" {
  rule = aws_cloudwatch_event_rule.sizer.name
  arn  = aws_lambda_function.sizer.arn
}

resource "aws_lambda_permission" "sizer" {
  statement_id  = "events"
  action        = "lambda:InvokeFunction"
  function_name = aws_lambda_function.sizer.function_name
  principal     = "events.amazonaws.com"
  source_arn    = aws_cloudwatch_event_rule.sizer.arn
}
