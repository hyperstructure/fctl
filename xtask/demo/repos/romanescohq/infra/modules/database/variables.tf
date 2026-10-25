variable "environment" {
  type = string
}

variable "subnet_ids" {
  type = list(string)
}

variable "instance_class" {
  type = string
}

variable "multi_az" {
  type = bool
}

variable "engine_version" {
  description = "The Postgres version the instance runs."
  type        = string
}

variable "storage_gb" {
  type    = number
  default = 100
}
