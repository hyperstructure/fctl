# The machines' image, where `ami_id` is unset: Ubuntu 24.04 with Docker,
# fctl and the sandbox image of `fctl_version` already pulled, built with
# EC2 Image Builder from deploy/cloud/runner, as Packer builds it from there.
# A new fctl version, a change to those files or a newer Ubuntu image
# builds a new one, which the group's instance refresh rolls out.

locals {
  build_image = var.ami_id == null
  runner_dir  = "${path.module}/../../runner"
  runner_files = {
    for f in [
      "provision.sh", "fctl-enroll", "fctl-drain", "fctl-firewall", "fctl-firewall.service",
      "fctl-scale-in.service", "fctl-scale-in-amazon-ebs", "fctl-retire-amazon-ebs",
      "fctl-size-group.service", "fctl-size-group.timer",
    ] : f => file("${local.runner_dir}/${f}")
  }
  component = yamlencode({
    schemaVersion = 1.0
    phases = [{
      name = "build"
      steps = concat(
        [for f, content in local.runner_files : {
          name   = "write-${replace(f, ".", "-")}"
          action = "CreateFile"
          inputs = [{ path = "/tmp/runner/${f}", content = content, overwrite = true }]
        }],
        [{
          name   = "provision"
          action = "ExecuteBash"
          inputs = { commands = ["FCTL_VERSION=${var.fctl_version} CLOUD=amazon-ebs sh /tmp/runner/provision.sh"] }
        }],
      )
    }]
  })
  # Image Builder takes each version of a name once: one from what it holds.
  image_version = "1.0.${parseint(substr(sha256(join("", [local.component, try(data.aws_ssm_parameter.ubuntu[0].insecure_value, "")])), 0, 6), 16)}"
}

data "aws_ssm_parameter" "ubuntu" {
  count = local.build_image ? 1 : 0
  name  = "/aws/service/canonical/ubuntu/server/24.04/stable/current/amd64/hvm/ebs-gp3/ami-id"
}

resource "aws_imagebuilder_component" "runner" {
  count    = local.build_image ? 1 : 0
  name     = "${var.name}-runner"
  platform = "Linux"
  version  = local.image_version
  data     = local.component
  tags     = var.tags

  lifecycle {
    create_before_destroy = true
  }
}

resource "aws_imagebuilder_image_recipe" "runner" {
  count        = local.build_image ? 1 : 0
  name         = "${var.name}-runner"
  version      = local.image_version
  parent_image = data.aws_ssm_parameter.ubuntu[0].insecure_value
  tags         = var.tags

  component {
    component_arn = aws_imagebuilder_component.runner[0].arn
  }

  block_device_mapping {
    device_name = "/dev/sda1"
    ebs {
      volume_size           = 30
      volume_type           = "gp3"
      delete_on_termination = true
    }
  }

  lifecycle {
    create_before_destroy = true
  }
}

# The machine Image Builder builds on: it reaches the internet through the
# runners' subnets, and Image Builder through Systems Manager.
resource "aws_iam_role" "builder" {
  count = local.build_image ? 1 : 0
  name  = "${var.name}-image-builder"
  tags  = var.tags
  assume_role_policy = jsonencode({
    Version   = "2012-10-17"
    Statement = [{ Effect = "Allow", Principal = { Service = "ec2.amazonaws.com" }, Action = "sts:AssumeRole" }]
  })
}

resource "aws_iam_role_policy_attachment" "builder" {
  for_each   = local.build_image ? toset(["AmazonSSMManagedInstanceCore", "EC2InstanceProfileForImageBuilder"]) : toset([])
  role       = aws_iam_role.builder[0].name
  policy_arn = "arn:${data.aws_partition.current.partition}:iam::aws:policy/${each.key}"
}

resource "aws_iam_instance_profile" "builder" {
  count = local.build_image ? 1 : 0
  name  = "${var.name}-image-builder"
  role  = aws_iam_role.builder[0].name
  tags  = var.tags
}

resource "aws_imagebuilder_infrastructure_configuration" "runner" {
  count                         = local.build_image ? 1 : 0
  name                          = "${var.name}-runner"
  instance_profile_name         = aws_iam_instance_profile.builder[0].name
  instance_types                = ["m7i.large"]
  subnet_id                     = var.subnet_ids[0]
  security_group_ids            = [aws_security_group.runner.id]
  terminate_instance_on_failure = true
  tags                          = var.tags

  instance_metadata_options {
    http_tokens                 = "required"
    http_put_response_hop_limit = 1
  }
}

resource "aws_imagebuilder_image" "runner" {
  count                            = local.build_image ? 1 : 0
  image_recipe_arn                 = aws_imagebuilder_image_recipe.runner[0].arn
  infrastructure_configuration_arn = aws_imagebuilder_infrastructure_configuration.runner[0].arn
  tags                             = merge(var.tags, { Name = "${var.name}-runner-${var.fctl_version}" })

  image_tests_configuration {
    image_tests_enabled = false
  }

  timeouts {
    create = "90m"
  }

  depends_on = [aws_iam_role_policy_attachment.builder]
}
