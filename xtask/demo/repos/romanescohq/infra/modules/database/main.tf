# The platform's Postgres, on RDS.

resource "aws_db_subnet_group" "main" {
  name       = "${var.environment}-platform"
  subnet_ids = var.subnet_ids
}

resource "aws_db_parameter_group" "main" {
  name   = "${var.environment}-postgres17"
  family = "postgres17"

  parameter {
    name  = "log_min_duration_statement"
    value = "500"
  }

  parameter {
    name         = "shared_preload_libraries"
    value        = "pg_stat_statements"
    apply_method = "pending-reboot"
  }
}

resource "aws_db_instance" "main" {
  identifier                   = "${var.environment}-platform"
  engine                       = "postgres"
  engine_version               = var.engine_version
  instance_class               = var.instance_class
  allocated_storage            = var.storage_gb
  storage_type                 = "gp3"
  multi_az                     = var.multi_az
  db_subnet_group_name         = aws_db_subnet_group.main.name
  parameter_group_name         = aws_db_parameter_group.main.name
  username                     = "platform"
  manage_master_user_password  = true
  backup_retention_period      = 14
  deletion_protection          = true
  performance_insights_enabled = true
  apply_immediately            = false
}
