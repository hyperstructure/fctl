# A pool of fctl runners on Azure, as docs/runners-in-your-cloud.md sets one
# up: a scale set of machines from the `fctl-runner` image in an Azure
# Compute Gallery (deploy/cloud's Packer template), each enrolling itself
# with an enrollment token from Key Vault, drained by the terminate
# notification before the scale set deletes it, and sizing the scale set to
# what the pool needs every minute (fctl-size-group.timer).

terraform {
  required_version = ">= 1.6"
  required_providers {
    azurerm = { source = "hashicorp/azurerm", version = ">= 5.0" }
  }
}

variable "subscription_id" {
  type = string
}

variable "resource_group" {
  description = "The resource group everything goes in, which this makes."
  type        = string
  default     = "fctl-runners"
}

variable "location" {
  type    = string
  default = "swedencentral"
}

variable "subnet_id" {
  description = "The runners' subnet, which reaches the internet through a NAT gateway."
  type        = string
}

variable "image_id" {
  description = "The image version Packer added to the gallery: /subscriptions/…/galleries/…/images/fctl-runner/versions/0.38.0."
  type        = string
}

variable "name" {
  description = "The scale set's name, and the prefix of everything else this makes."
  type        = string
  default     = "fctl-runners"
}

variable "vault_name" {
  description = "The Key Vault's name, unique in Azure."
  type        = string
}

variable "fctl_server" {
  description = "The fctl instance the machines enroll with."
  type        = string
}

variable "enrollment_token" {
  description = "The enrollment token (fctl_et_…), which goes into Key Vault. It is in Terraform's state too: keep that state as safe."
  type        = string
  sensitive   = true
}

variable "admin_ssh_key" {
  description = "A public key for the machines' admin user, which Azure requires."
  type        = string
}

variable "vm_size" {
  type    = string
  default = "Standard_D8s_v5"
}

variable "disk_gb" {
  type    = number
  default = 256
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
  description = "The fewest machines, 1 or more: the machines size the scale set, so with none nothing would."
  type        = number
  default     = 1
}

provider "azurerm" {
  features {}
  subscription_id = var.subscription_id
}

data "azurerm_client_config" "current" {}

resource "azurerm_resource_group" "runners" {
  name     = var.resource_group
  location = var.location
}

# A vault of the runners' own, which only roles open.
resource "azurerm_key_vault" "runners" {
  name                       = var.vault_name
  resource_group_name        = azurerm_resource_group.runners.name
  location                   = azurerm_resource_group.runners.location
  tenant_id                  = data.azurerm_client_config.current.tenant_id
  sku_name                   = "standard"
  rbac_authorization_enabled = true
}

# Whoever applies this writes the token.
resource "azurerm_role_assignment" "writer" {
  scope                = azurerm_key_vault.runners.id
  role_definition_name = "Key Vault Secrets Officer"
  principal_id         = data.azurerm_client_config.current.object_id
}

resource "azurerm_key_vault_secret" "token" {
  name         = "enroll-token"
  value        = var.enrollment_token
  key_vault_id = azurerm_key_vault.runners.id
  depends_on   = [azurerm_role_assignment.writer]
}

resource "azurerm_linux_virtual_machine_scale_set" "runners" {
  name                = var.name
  resource_group_name = azurerm_resource_group.runners.name
  location            = azurerm_resource_group.runners.location
  sku                 = var.vm_size
  instances           = var.min_size
  admin_username      = "fctl-admin"
  source_image_id     = var.image_id
  secure_boot_enabled = true
  vtpm_enabled        = true
  upgrade_mode        = "Manual"
  custom_data = base64encode(templatefile("${path.module}/cloud-init.sh.tftpl", {
    vault_uri = azurerm_key_vault.runners.vault_uri
    server    = var.fctl_server
    min_size  = var.min_size
    capacity  = var.capacity
    cpus      = var.cpus
    memory    = var.memory
  }))

  admin_ssh_key {
    username   = "fctl-admin"
    public_key = var.admin_ssh_key
  }

  os_disk {
    caching              = "ReadWrite"
    storage_account_type = "Premium_LRS"
    disk_size_gb         = var.disk_gb
  }

  network_interface {
    name    = "runner"
    primary = true
    ip_configuration {
      name      = "runner"
      primary   = true
      subnet_id = var.subnet_id
    }
  }

  identity {
    type = "SystemAssigned"
  }

  # The scale set tells a machine it deletes 15 minutes ahead, the most
  # Azure gives: its watcher drains it meanwhile.
  termination_notification {
    enabled = true
    timeout = "PT15M"
  }

  # The machines size the scale set from the pool's demand.
  lifecycle {
    ignore_changes = [instances]
  }

  depends_on = [azurerm_key_vault_secret.token]
}

# The machines read their token and resize their scale set: nothing else.
resource "azurerm_role_assignment" "reader" {
  scope                = azurerm_key_vault.runners.id
  role_definition_name = "Key Vault Secrets User"
  principal_id         = azurerm_linux_virtual_machine_scale_set.runners.identity[0].principal_id
}

resource "azurerm_role_assignment" "resize" {
  scope                = azurerm_linux_virtual_machine_scale_set.runners.id
  role_definition_name = "Virtual Machine Contributor"
  principal_id         = azurerm_linux_virtual_machine_scale_set.runners.identity[0].principal_id
}

output "scale_set" {
  value = azurerm_linux_virtual_machine_scale_set.runners.name
}
