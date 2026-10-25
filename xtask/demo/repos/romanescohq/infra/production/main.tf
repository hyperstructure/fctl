# Production, in eu-north-1.

module "network" {
  source      = "../modules/network"
  environment = "production"
  cidr        = "10.20.0.0/16"
}

module "database" {
  source         = "../modules/database"
  environment    = "production"
  subnet_ids     = module.network.private_subnet_ids
  instance_class = "db.r7g.xlarge"
  multi_az       = true
  engine_version = "17.6"
  storage_gb     = 400
}

module "platform" {
  source        = "../modules/platform"
  environment   = "production"
  desired_count = 4
  worker_count  = 3
  database_url  = module.database.url
}

module "monitoring" {
  source         = "../modules/monitoring"
  environment    = "production"
  load_balancer  = module.platform.load_balancer_suffix
  database_id    = module.database.id
  pager_endpoint = var.pager_endpoint
}

variable "pager_endpoint" {
  description = "The pager's HTTPS endpoint, from the parameter store."
  type        = string
}
