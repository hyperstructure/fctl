variable "environment" {
  type = string
}

variable "load_balancer" {
  description = "The platform API's load balancer, as CloudWatch names it (app/…)."
  type        = string
}

variable "database_id" {
  type = string
}

variable "pager_endpoint" {
  description = "Where the on-call topic pages: the pager's HTTPS endpoint. None in staging."
  type        = string
  default     = null
}

variable "api_5xx_per_minute" {
  type    = number
  default = 20
}
