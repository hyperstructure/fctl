variable "name" {
  description = "The group's name, and the prefix of everything else this module makes."
  type        = string
  default     = "fctl-runners"
}

variable "fctl_server" {
  description = "The fctl instance the machines enroll with: https://app.fctl.sh or one of your own."
  type        = string
}

variable "enrollment_token" {
  description = "The enrollment token (fctl_et_…) the machines enroll with, which goes into Secrets Manager. It is in Terraform's state too: keep that state as safe."
  type        = string
  sensitive   = true
}

variable "fctl_version" {
  description = "The fctl version the image has, the one the instance runs (its /api/v1/health says): 0.38.0, say. Hosts update themselves after."
  type        = string
}

variable "ami_id" {
  description = "An image built with deploy/cloud's Packer template. Unset: this module builds one with EC2 Image Builder, in about 20 minutes."
  type        = string
  default     = null
}

variable "vpc_id" {
  description = "The VPC of the subnets."
  type        = string
}

variable "subnet_ids" {
  description = "Private subnets that reach the internet through a NAT gateway: the machines take no inbound connections."
  type        = list(string)
}

variable "instance_type" {
  description = "The machines' type: each runs `capacity` sandboxes of `cpus` and `memory` at once."
  type        = string
  default     = "m7i.2xlarge"
}

variable "disk_gb" {
  description = "Each machine's disk: the images, the sandboxes, the package caches and the 20 GB the host keeps free."
  type        = number
  default     = 200
}

variable "capacity" {
  description = "Sessions each machine runs at once."
  type        = number
  default     = 3
}

variable "cpus" {
  description = "Each sandbox's CPUs, when its project sets none."
  type        = number
  default     = 4
}

variable "memory" {
  description = "Each sandbox's memory, when its project sets none."
  type        = string
  default     = "8g"
}

variable "min_size" {
  description = "The fewest machines: 0 lets the group empty when nothing runs, and the first session then waits for one to boot."
  type        = number
  default     = 1
}

variable "max_size" {
  description = "The most machines: keep the enrollment token's --max-runners a few above it."
  type        = number
  default     = 20
}

variable "retire_idle_minutes" {
  description = "Minutes a machine's runner runs nothing before the machine takes itself out of the group, which then never takes a busy one (its machines are protected from scale-in). 0: the sizer lowers the group too, and the group picks which machine goes."
  type        = number
  default     = 20
}

variable "drain_hours" {
  description = "How long a machine the group takes may drain, 1 or 2 hours (the drain hook's longest heartbeat): its turns under way finish within it, less 5 minutes for what follows."
  type        = number
  default     = 2

  validation {
    condition     = contains([1, 2], var.drain_hours)
    error_message = "drain_hours is 1 or 2: the drain hook holds a machine for 2 hours at most."
  }
}

variable "tags" {
  description = "Tags for everything this module makes."
  type        = map(string)
  default     = {}
}
