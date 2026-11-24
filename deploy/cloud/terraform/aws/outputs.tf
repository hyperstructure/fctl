output "group_name" {
  description = "The Auto Scaling group's name."
  value       = aws_autoscaling_group.runners.name
}

output "ami_id" {
  description = "The image the machines start from."
  value       = local.ami
}

output "secret_arn" {
  description = "The Secrets Manager secret the machines read the enrollment token from: put a new token there before the old one expires."
  value       = aws_secretsmanager_secret.token.arn
}

output "role_name" {
  description = "The machines' IAM role, to which a policy can add what their sessions' hosts need."
  value       = aws_iam_role.runner.name
}
