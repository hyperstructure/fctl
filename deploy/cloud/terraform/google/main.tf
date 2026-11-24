# A pool of fctl runners on Google Cloud, as docs/runners-in-your-cloud.md
# sets one up: a regional managed instance group of machines from the
# `fctl-runner` image family (deploy/cloud's Packer template), each enrolling
# itself with an enrollment token from Secret Manager, drained by graceful
# shutdown before the group deletes it, and sizing the group to what the pool
# needs every minute (fctl-size-group.timer).

terraform {
  required_version = ">= 1.6"
  required_providers {
    google      = { source = "hashicorp/google", version = ">= 8.0" }
    google-beta = { source = "hashicorp/google-beta", version = ">= 8.0" }
  }
}

variable "project" {
  description = "The project the runners run in."
  type        = string
}

variable "region" {
  description = "The group's region, whose subnet reaches the internet through Cloud NAT."
  type        = string
  default     = "europe-north1"
}

variable "subnetwork" {
  description = "The subnet the machines go in (a self link or a name), without external addresses."
  type        = string
}

variable "name" {
  description = "The group's name, and the prefix of everything else this makes."
  type        = string
  default     = "fctl-runners"
}

variable "fctl_server" {
  description = "The fctl instance the machines enroll with."
  type        = string
}

variable "enrollment_token" {
  description = "The enrollment token (fctl_et_…), which goes into Secret Manager. It is in Terraform's state too: keep that state as safe."
  type        = string
  sensitive   = true
}

variable "machine_type" {
  type    = string
  default = "n2-standard-8"
}

variable "disk_gb" {
  type    = number
  default = 200
}

variable "capacity" {
  description = "Sessions each machine runs at once."
  type        = number
  default     = 3
}

variable "cpus" {
  type    = number
  default = 4
}

variable "memory" {
  type    = string
  default = "8g"
}

variable "min_size" {
  description = "The fewest machines, 1 or more: the machines size the group, so with none nothing would."
  type        = number
  default     = 1
}

provider "google" {
  project = var.project
  region  = var.region
}

# Instance templates take graceful shutdown only through google-beta.
provider "google-beta" {
  project = var.project
  region  = var.region
}

resource "google_secret_manager_secret" "token" {
  secret_id = "${var.name}-enroll-token"
  replication {
    auto {}
  }
}

resource "google_secret_manager_secret_version" "token" {
  secret      = google_secret_manager_secret.token.id
  secret_data = var.enrollment_token
}

# The machines read their token and resize their group: nothing else.
resource "google_service_account" "runner" {
  account_id   = var.name
  display_name = "fctl runners"
}

resource "google_secret_manager_secret_iam_member" "runner" {
  secret_id = google_secret_manager_secret.token.id
  role      = "roles/secretmanager.secretAccessor"
  member    = "serviceAccount:${google_service_account.runner.email}"
}

resource "google_project_iam_custom_role" "resize" {
  role_id     = replace("${var.name}_resize", "-", "_")
  title       = "Resize fctl runner groups"
  permissions = ["compute.instanceGroupManagers.get", "compute.instanceGroupManagers.update"]
}

resource "google_project_iam_member" "resize" {
  project = var.project
  role    = google_project_iam_custom_role.resize.id
  member  = "serviceAccount:${google_service_account.runner.email}"
}

data "google_compute_image" "runner" {
  family = "fctl-runner"
}

resource "google_compute_instance_template" "runner" {
  provider     = google-beta
  name_prefix  = "${var.name}-"
  machine_type = var.machine_type

  disk {
    source_image = data.google_compute_image.runner.self_link
    disk_size_gb = var.disk_gb
    disk_type    = "pd-balanced"
    boot         = true
    auto_delete  = true
  }

  network_interface {
    subnetwork = var.subnetwork
  }

  service_account {
    email  = google_service_account.runner.email
    scopes = ["cloud-platform"]
  }

  shielded_instance_config {
    enable_secure_boot          = true
    enable_vtpm                 = true
    enable_integrity_monitoring = true
  }

  # Graceful shutdown gives a machine the group deletes two hours to drain.
  scheduling {
    graceful_shutdown {
      enabled = true
      max_duration {
        seconds = 7200
      }
    }
  }


  metadata = {
    enable-oslogin         = "TRUE"
    block-project-ssh-keys = "TRUE"
    startup-script = templatefile("${path.module}/startup.sh.tftpl", {
      secret   = google_secret_manager_secret.token.secret_id
      server   = var.fctl_server
      min_size = var.min_size
      capacity = var.capacity
      cpus     = var.cpus
      memory   = var.memory
    })
  }

  lifecycle {
    create_before_destroy = true
  }
}

resource "google_compute_region_instance_group_manager" "runners" {
  name               = var.name
  base_instance_name = var.name
  region             = var.region
  target_size        = var.min_size

  version {
    instance_template = google_compute_instance_template.runner.self_link_unique
  }

  update_policy {
    type                  = "PROACTIVE"
    minimal_action        = "REPLACE"
    max_surge_fixed       = 3
    max_unavailable_fixed = 0
  }

  # The machines size the group from the pool's demand.
  lifecycle {
    ignore_changes = [target_size]
  }

  depends_on = [google_secret_manager_secret_version.token, google_secret_manager_secret_iam_member.runner]
}

output "group" {
  value = google_compute_region_instance_group_manager.runners.name
}
