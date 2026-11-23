# A pool of fctl runners on AWS: an Auto Scaling group of machines from an
# image with Docker, fctl and the sandbox image, each enrolling itself as one
# of a workspace's runners with an enrollment token from Secrets Manager.
# docs/runners-in-your-cloud.md walks through the same steps by hand.

data "aws_region" "current" {}
data "aws_caller_identity" "current" {}
data "aws_partition" "current" {}

locals {
  ami    = coalesce(var.ami_id, try(one(one(aws_imagebuilder_image.runner[0].output_resources).amis).image, null))
  retire = var.retire_idle_minutes > 0
  group_arn = format(
    "arn:%s:autoscaling:%s:%s:autoScalingGroup:*:autoScalingGroupName/%s",
    data.aws_partition.current.partition, data.aws_region.current.region, data.aws_caller_identity.current.account_id, var.name
  )
}

# No inbound rules: nothing connects to the machines.
resource "aws_security_group" "runner" {
  name        = var.name
  description = "fctl runners: outbound only"
  vpc_id      = var.vpc_id
  tags        = var.tags

  egress {
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }
}

resource "aws_secretsmanager_secret" "token" {
  name                    = "${var.name}/enroll-token"
  description             = "The enrollment token fctl runners enroll with"
  recovery_window_in_days = 0
  tags                    = var.tags
}

resource "aws_secretsmanager_secret_version" "token" {
  secret_id     = aws_secretsmanager_secret.token.id
  secret_string = var.enrollment_token
}

# The machines read their token and end the drain hook for themselves, and
# take themselves out of the group once idle: nothing else.
resource "aws_iam_role" "runner" {
  name = var.name
  tags = var.tags
  assume_role_policy = jsonencode({
    Version   = "2012-10-17"
    Statement = [{ Effect = "Allow", Principal = { Service = "ec2.amazonaws.com" }, Action = "sts:AssumeRole" }]
  })
}

resource "aws_iam_role_policy" "runner" {
  name = var.name
  role = aws_iam_role.runner.id
  policy = jsonencode({
    Version = "2012-10-17"
    Statement = concat(
      [
        { Effect = "Allow", Action = "secretsmanager:GetSecretValue", Resource = aws_secretsmanager_secret.token.arn },
        {
          Effect   = "Allow"
          Action   = ["autoscaling:CompleteLifecycleAction", "autoscaling:RecordLifecycleActionHeartbeat"]
          Resource = local.group_arn
        },
      ],
      local.retire ? [
        { Effect = "Allow", Action = "autoscaling:TerminateInstanceInAutoScalingGroup", Resource = local.group_arn }
      ] : [],
    )
  })
}

resource "aws_iam_instance_profile" "runner" {
  name = var.name
  role = aws_iam_role.runner.name
  tags = var.tags
}

resource "aws_launch_template" "runner" {
  name                   = var.name
  image_id               = local.ami
  instance_type          = var.instance_type
  vpc_security_group_ids = [aws_security_group.runner.id]
  tags                   = var.tags
  user_data = base64encode(templatefile("${path.module}/user-data.sh.tftpl", {
    group    = var.name
    secret   = aws_secretsmanager_secret.token.name
    server   = var.fctl_server
    capacity = var.capacity
    cpus     = var.cpus
    memory   = var.memory
    retire   = var.retire_idle_minutes
  }))

  iam_instance_profile {
    name = aws_iam_instance_profile.runner.name
  }

  # IMDSv2 with one hop: containers don't reach the metadata service.
  metadata_options {
    http_endpoint               = "enabled"
    http_tokens                 = "required"
    http_put_response_hop_limit = 1
  }

  block_device_mappings {
    device_name = "/dev/sda1"
    ebs {
      volume_size           = var.disk_gb
      volume_type           = "gp3"
      delete_on_termination = true
    }
  }

  tag_specifications {
    resource_type = "instance"
    tags          = merge(var.tags, { Name = var.name })
  }
}

resource "aws_autoscaling_group" "runners" {
  name                = var.name
  min_size            = var.min_size
  max_size            = var.max_size
  vpc_zone_identifier = var.subnet_ids
  # Machines that retire take themselves out; the group never picks one.
  protect_from_scale_in = local.retire

  launch_template {
    id      = aws_launch_template.runner.id
    version = aws_launch_template.runner.latest_version
  }

  # The drain hook: a machine the group takes, or that retires, waits up to
  # `drain_hours` for its watcher to say its turns ended.
  initial_lifecycle_hook {
    name                 = "fctl-drain"
    lifecycle_transition = "autoscaling:EC2_INSTANCE_TERMINATING"
    heartbeat_timeout    = var.drain_hours * 3600
    default_result       = "CONTINUE"
  }

  # A new image (a new fctl version) replaces the machines, each through
  # the drain hook.
  instance_refresh {
    strategy = "Rolling"
    preferences {
      min_healthy_percentage       = 50
      scale_in_protected_instances = "Refresh"
    }
  }

  # The sizer sets the size from the pool's demand.
  lifecycle {
    ignore_changes = [desired_capacity]
  }

  dynamic "tag" {
    for_each = merge(var.tags, { Name = var.name })
    content {
      key                 = tag.key
      value               = tag.value
      propagate_at_launch = true
    }
  }
}
