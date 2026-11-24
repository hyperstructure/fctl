packer {
  required_plugins {
    amazon        = { version = ">= 1.8.4", source = "github.com/hashicorp/amazon" }
    googlecompute = { version = ">= 1.2.9", source = "github.com/hashicorp/googlecompute" }
    azure         = { version = ">= 2.6.5", source = "github.com/hashicorp/azure" }
  }
}

variable "fctl_version" { type = string }
variable "aws_region" { default = "eu-north-1" }
variable "gcp_project" { default = "acme-runners" }
variable "gcp_zone" { default = "europe-north1-a" }
variable "azure_subscription" { default = "" }
variable "azure_resource_group" { default = "acme-runners" }
variable "azure_location" { default = "swedencentral" }
variable "azure_gallery" { default = "acme_runners" }

locals { name = "fctl-runner-${replace(var.fctl_version, ".", "-")}" }

source "amazon-ebs" "fctl-runner" {
  region        = var.aws_region
  instance_type = "m7i.large"
  ssh_username  = "ubuntu"
  ami_name      = "${local.name}-{{timestamp}}"
  imds_support  = "v2.0"
  source_ami_filter {
    filters = {
      name                = "ubuntu/images/hvm-ssd-gp3/ubuntu-noble-24.04-amd64-server-*"
      root-device-type    = "ebs"
      virtualization-type = "hvm"
    }
    owners      = ["099720109477"] # Canonical
    most_recent = true
  }
  launch_block_device_mappings {
    device_name           = "/dev/sda1"
    volume_size           = 30
    volume_type           = "gp3"
    delete_on_termination = true
  }
}

source "googlecompute" "fctl-runner" {
  project_id              = var.gcp_project
  zone                    = var.gcp_zone
  source_image_family     = "ubuntu-2404-lts-amd64"
  source_image_project_id = ["ubuntu-os-cloud"]
  machine_type            = "e2-standard-2"
  disk_size               = 30
  ssh_username            = "packer"
  image_name              = "${local.name}-{{timestamp}}"
  image_family            = "fctl-runner"
  enable_secure_boot      = true
}

source "azure-arm" "fctl-runner" {
  use_azure_cli_auth = true
  subscription_id    = var.azure_subscription
  location           = var.azure_location
  vm_size            = "Standard_D2s_v5"
  os_type            = "Linux"
  image_publisher    = "Canonical"
  image_offer        = "ubuntu-24_04-lts"
  image_sku          = "server"
  shared_image_gallery_destination {
    subscription   = var.azure_subscription
    resource_group = var.azure_resource_group
    gallery_name   = var.azure_gallery
    image_name     = "fctl-runner"
    image_version  = var.fctl_version
    target_region { name = var.azure_location }
  }
}

build {
  sources = ["source.amazon-ebs.fctl-runner", "source.googlecompute.fctl-runner", "source.azure-arm.fctl-runner"]

  provisioner "file" {
    source      = "runner"
    destination = "/tmp"
  }
  provisioner "shell" {
    environment_vars = ["FCTL_VERSION=${var.fctl_version}", "CLOUD=${source.type}"]
    execute_command  = "chmod +x {{ .Path }}; {{ .Vars }} sudo -E sh '{{ .Path }}'"
    script           = "runner/provision.sh"
  }
  provisioner "shell" {
    only            = ["azure-arm.fctl-runner"]
    execute_command = "chmod +x {{ .Path }}; {{ .Vars }} sudo -E sh '{{ .Path }}'"
    inline          = ["/usr/sbin/waagent -force -deprovision+user && export HISTSIZE=0 && sync"]
  }
}
