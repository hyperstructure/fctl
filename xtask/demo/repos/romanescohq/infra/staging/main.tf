# Staging, in eu-north-1: production's shape, smaller.

module "network" {
  source      = "../modules/network"
  environment = "staging"
  cidr        = "10.30.0.0/16"
}

module "database" {
  source         = "../modules/database"
  environment    = "staging"
  subnet_ids     = module.network.private_subnet_ids
  instance_class = "db.t4g.large"
  multi_az       = false
  engine_version = "17.6"
}

module "platform" {
  source        = "../modules/platform"
  environment   = "staging"
  desired_count = 1
  worker_count  = 1
  database_url  = module.database.url
}

module "monitoring" {
  source        = "../modules/monitoring"
  environment   = "staging"
  load_balancer = module.platform.load_balancer_suffix
  database_id   = module.database.id
}
