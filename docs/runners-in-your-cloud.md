# Runners in your cloud

At the end of this guide, Acme's sessions run on a group of machines in
its own AWS, Google Cloud or Azure account. Each machine starts from an
image with Docker, fctl and the sandbox image ready, enrolls itself as
one of the workspace `acme`'s runners within a minute or two of booting,
and takes sessions at once. When the group shrinks, the machine it
removes first lets its agents finish their turns, then leaves the
workspace's **Runners** tab, and only then does the cloud delete it.

An admin of `acme` makes the enrollment token the machines use, in
step 1. The rest takes someone who can create images, roles, secrets and
autoscaling groups in the cloud account, and about two hours, half of
them waiting for builds.

```text
the group          the machine                     fctl
scales out ──────▶ boots from the image
                   reads the token
                   fctl host install ────────────▶ a runner, online:
                                                   sessions start
scales in ───────▶ the watcher hears of it
                   fctl host uninstall --wait ───▶ no new sessions
                   turns end, idle sessions stop
                   the runner goes ──────────────▶ off the Runners tab
deletes it ◀────── "go on"
```

The diagram: a machine the group adds boots from the image, reads the
enrollment token from the cloud's secret store and enrolls as a runner.
A machine the group takes out learns of it from the cloud, drains:
fctl starts no new session there, and the turns under way finish. Then
the machine removes its runner and tells the cloud to go on.

## What you need

- An fctl instance the machines reach, app.fctl.sh or one of your own
  (`https://fctl.acme.dev` here), and its version: hosts keep to the
  version their control plane runs.
- An admin of the workspace, for the enrollment token.
- In the cloud account: the rights to create machine images, roles or
  service accounts, a secret, templates and autoscaling groups, and the
  cloud's CLI signed in (`aws`, `gcloud` or `az`).
- A network for the machines with outbound internet access through a
  NAT gateway, and none inbound: they reach your fctl instance, GitHub,
  Anthropic, ghcr.io, fctl.sh and the package registries your projects
  use. Nothing connects to them.
- [Packer](https://developer.hashicorp.com/packer/install) on the
  machine you build the image from.

## Your values

Set these once, in the shell you run the commands in. Each cloud's
section adds its own:

```sh
export FCTL_SERVER=https://fctl.acme.dev
export FCTL_WORKSPACE=acme
export FCTL_VERSION="$(curl -s "$FCTL_SERVER/api/v1/health" | jq -r .version)"   # 0.37.1, say
```

## 1. Make an enrollment token

An admin of `acme`, on the workspace's **Runners** tab under **Enrollment
tokens**, or with the CLI:

```sh
fctl runners enrollment-tokens create "Cloud fleet" -w "$FCTL_WORKSPACE" \
  --max-runners 25 --remove-offline-after 15m --expires 1y > enroll-token
```

It writes the token (`fctl_et_…`) to `enroll-token`, and only there: it's
shown once. The machines it enrolls run everyone's sessions in `acme`,
and no other workspace's.

- **`--max-runners`**: a few more than the group's largest size. A
  runner whose machine went without draining counts until fctl removes
  it.
- **`--remove-offline-after 15m`** removes such a runner, of a machine a
  Spot reclaim or a crash took. Keep it at 15 minutes or more: a runner
  removed during a network outage can't come back, and its machine then
  runs nothing.
- **`--expires 1y`**: before then, make another and put it in the secret
  store (step 4 of your cloud). Machines that enrolled keep their runner
  tokens, so they carry on.

**Check:** `fctl runners enrollment-tokens -w acme` lists it, with
"Enrolls up to 25 runners at a time, which run everyone's sessions at
once and are removed once they have been offline for 15 minutes."

## 2. Choose the machines' size

Each machine runs `capacity` sessions at once, each sandbox with up to
`cpus` and `memory` of its own. The sandboxes' memory together must fit
in the machine's, with room left for the host, Docker and the page
cache; CPUs can be shared, as sessions spend most of their time waiting
for Claude. The disk holds the images, the sandboxes (kept 72 hours after
their session stops, so that a resume carries on there), the package
caches (up to 10 GB a project) and the 20 GB the host keeps free.

An 8-vCPU, 32 GiB machine with a 200 GB disk runs 3 sessions of 4 CPUs
and 8 GiB each: 24 GiB for sandboxes, 8 GiB for the rest.

| | AWS | Google Cloud | Azure |
|---|---|---|---|
| 8 vCPUs, 32 GiB | `m7i.2xlarge` | `n2-standard-8` | `Standard_D8s_v5` |
| The disk | 200 GB `gp3` | 200 GB `pd-balanced` | 256 GB Premium SSD |

A project that sets more for its sandboxes (`fctl projects set --cpus 8
--memory 16g`) gets that, up to the whole machine: give such projects
machines of their own size, a second group whose token has a label they
ask for ([Runner pools](cli.md#runner-pools)), or keep `capacity` lower.

## 3. Write the machine's scripts

The image needs a handful of files, the same on every cloud. They go in
a directory, `runner/`, beside the Packer template of step 4.

`runner/provision.sh` prepares the image, as root:

```sh
#!/bin/sh
# Prepares an fctl runner's machine image: Docker, the host's user, fctl at
# the instance's version, the sandbox image of that version, and the scripts
# that enroll the machine and drain it. FCTL_VERSION and CLOUD come from Packer.
set -eu
export DEBIAN_FRONTEND=noninteractive

# Docker Engine, from Docker's repository.
apt-get update
apt-get install -y ca-certificates curl jq unzip
install -m 0755 -d /etc/apt/keyrings
curl -fsSL https://download.docker.com/linux/ubuntu/gpg -o /etc/apt/keyrings/docker.asc
chmod a+r /etc/apt/keyrings/docker.asc
cat > /etc/apt/sources.list.d/docker.sources <<EOF
Types: deb
URIs: https://download.docker.com/linux/ubuntu
Suites: $(. /etc/os-release && echo "${UBUNTU_CODENAME:-$VERSION_CODENAME}")
Components: stable
Architectures: $(dpkg --print-architecture)
Signed-By: /etc/apt/keyrings/docker.asc
EOF
apt-get update
apt-get install -y docker-ce docker-ce-cli containerd.io
systemctl enable docker

# AWS's CLI, which the boot script reads the token with; the other clouds' need none.
if [ "$CLOUD" = amazon-ebs ]; then
  curl -fsSL "https://awscli.amazonaws.com/awscli-exe-linux-$(uname -m).zip" -o /tmp/awscli.zip
  unzip -q /tmp/awscli.zip -d /tmp && /tmp/aws/install
fi

# Sandboxes don't reach the cloud's metadata service, only the DNS it serves.
install -m 0755 /tmp/runner/fctl-firewall /tmp/runner/fctl-enroll /tmp/runner/fctl-drain /usr/local/sbin/
install -m 0755 "/tmp/runner/fctl-scale-in-$CLOUD" /usr/local/sbin/fctl-scale-in
install -m 0644 /tmp/runner/fctl-firewall.service /tmp/runner/fctl-scale-in.service /etc/systemd/system/
systemctl enable fctl-firewall.service fctl-scale-in.service

# The host's user, in the docker group, whose systemd manager starts at boot.
useradd --create-home --shell /bin/bash fctl
usermod -aG docker fctl
loginctl enable-linger fctl

# fctl, in a directory the host's user writes to, where the host updates it.
runuser -u fctl -- sh -c 'cd && curl -fsSL https://fctl.sh/install | FCTL_NO_COMPLETIONS=1 sh'

# The sandbox image of that version, so that a new machine's first session doesn't wait for it.
docker pull "ghcr.io/hyperstructure/sandbox:$FCTL_VERSION"
```

`runner/fctl-enroll` enrolls the machine, at its first boot, with the
token on stdin:

```sh
#!/bin/sh
# fctl-enroll SERVER NAME [fctl host install's options] < token
set -eu
server=$1 name=$2
shift 2
token=$(cat)
uid=$(id -u fctl)
systemctl start "user@$uid.service" # the host's systemd manager, which lingering starts at boot
runuser -u fctl -- env XDG_RUNTIME_DIR="/run/user/$uid" FCTL_SERVER="$server" FCTL_ENROLL_TOKEN="$token" \
  /home/fctl/.local/bin/fctl host install --name "$name" "$@"
```

`runner/fctl-drain` drains it, when the cloud is about to take it:

```sh
#!/bin/sh
# fctl-drain SECONDS: no new sessions start here; the turns under way get SECONDS
# to end, then what still runs stops, and its agents push what they committed.
# Then the runner is removed.
set -u
fctl="runuser -u fctl -- env XDG_RUNTIME_DIR=/run/user/$(id -u fctl) /home/fctl/.local/bin/fctl"
timeout "$1" $fctl host uninstall --wait || $fctl host uninstall --force
```

Hosts after fctl 0.37.1 keep Docker and Firecracker sandboxes off the
cloud's metadata service (`169.254.169.254`) themselves, where they would
find the machine's own credentials and its boot script
([Security](security.md#sandboxes)). `runner/fctl-firewall`, with
`runner/fctl-firewall.service` that runs it before Docker starts, adds a
fence of the machine's own: it also keeps containers off Azure's
WireServer (`168.63.129.16`), which the host leaves alone, and keeps an
older host's sandboxes off both. Google Cloud and Azure serve DNS from
those addresses, so that stays open:

```sh
#!/bin/sh
# Keeps containers from the cloud's metadata services, but not from the DNS they serve.
set -eu
iptables -N DOCKER-USER 2>/dev/null || true
for address in 169.254.169.254 168.63.129.16; do
  for protocol in tcp udp; do
    rule="-d $address -p $protocol ! --dport 53 -j REJECT"
    iptables -C DOCKER-USER $rule 2>/dev/null || iptables -I DOCKER-USER $rule
  done
done
```

```ini
[Unit]
Description=Keep fctl's sandboxes from the cloud's metadata services
Before=docker.service

[Service]
Type=oneshot
RemainAfterExit=yes
ExecStart=/usr/local/sbin/fctl-firewall

[Install]
RequiredBy=docker.service
```

`runner/fctl-scale-in.service` runs the watcher that drains the machine
before the cloud takes it, `fctl-scale-in`, one for each cloud
(`runner/fctl-scale-in-amazon-ebs`, `-googlecompute` and `-azure-arm`, in
the cloud's section):

```ini
[Unit]
Description=Drain this fctl runner before the cloud takes the machine
Wants=network-online.target
After=network-online.target docker.service

[Service]
EnvironmentFile=-/etc/default/fctl-scale-in
ExecStart=/usr/local/sbin/fctl-scale-in
Restart=on-failure
RestartSec=10

[Install]
WantedBy=multi-user.target
```

## 4. The image

`fctl-runner.pkr.hcl`, beside `runner/`, builds the image on each cloud
from Ubuntu 24.04, with the scripts above:

```hcl
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
    gallery_name   = "acme_runners"
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
```

```sh
packer init .
```

Each cloud's section builds it there with `packer build -only=…`. For
Arm machines (Graviton, Axion, Cobalt), use the clouds' `arm64` Ubuntu
images and Arm machine types: fctl and the sandbox image run on both.

**No Packer?** Start a machine from Ubuntu 24.04, copy `runner/` to
`/tmp/runner` on it, run `sudo FCTL_VERSION=… CLOUD=amazon-ebs sh
/tmp/runner/provision.sh` (`googlecompute` or `azure-arm` on the
others), then make an image of its disk: `aws ec2 create-image`,
`gcloud compute images create --source-disk`, or Azure's `az sig
image-version create` after `waagent -deprovision+user`.

**No image at all?** The boot script can run `provision.sh` itself
before it enrolls, from a copy in a bucket. Each new machine then takes
a few minutes longer to take sessions: it installs Docker and fctl and
pulls the sandbox image, about 1.4 GB.

## 5. On AWS

```sh
export AWS_REGION=eu-north-1 GROUP=acme-runners
export ACCOUNT="$(aws sts get-caller-identity --query Account --output text)"
export SUBNETS=subnet-0a1b2c3d,subnet-4e5f6a7b   # private subnets, with a NAT gateway
export SECURITY_GROUP=sg-0c1d2e3f                # no inbound rules
```

### The image

```sh
packer build -only='amazon-ebs.fctl-runner' -var "fctl_version=$FCTL_VERSION" -var "aws_region=$AWS_REGION" .
```

It takes about 15 minutes, and ends with the AMI's id (`ami-…`): keep it
as `AMI`.

`runner/fctl-scale-in-amazon-ebs`, the watcher, waits for Auto Scaling
to take the machine, which its metadata says through the lifecycle hook
below, or for a Spot reclaim's two minutes' notice:

```sh
#!/bin/sh
# Drains this runner when Auto Scaling takes the machine, then lets it go on.
set -u
until [ -f /etc/default/fctl-scale-in ]; do sleep 5; done
. /etc/default/fctl-scale-in # GROUP and HOOK, from the boot script
md() {
  t=$(curl -sf -X PUT http://169.254.169.254/latest/api/token -H 'X-aws-ec2-metadata-token-ttl-seconds: 60') &&
    curl -sf -H "X-aws-ec2-metadata-token: $t" "http://169.254.169.254/latest/meta-data/$1"
}
while :; do
  if md spot/instance-action >/dev/null; then deadline=45; break; fi # two minutes' notice
  if [ "$(md autoscaling/target-lifecycle-state)" = Terminated ]; then deadline=6900; break; fi # the hook's two hours
  sleep 5
done
fctl-drain "$deadline"
aws autoscaling complete-lifecycle-action --region "$(md placement/region)" --auto-scaling-group-name "$GROUP" \
  --lifecycle-hook-name "$HOOK" --instance-id "$(md instance-id)" --lifecycle-action-result CONTINUE || true
```

### The token, and what the machines may do

The token goes into Secrets Manager, and the machines' role reads that
secret and nothing else, and ends the group's lifecycle hook for itself:

```sh
aws secretsmanager create-secret --name fctl/enroll-token --secret-string file://enroll-token
cat > role-policy.json <<EOF
{
  "Version": "2012-10-17",
  "Statement": [
    { "Effect": "Allow", "Action": "secretsmanager:GetSecretValue",
      "Resource": "arn:aws:secretsmanager:$AWS_REGION:$ACCOUNT:secret:fctl/enroll-token-??????" },
    { "Effect": "Allow",
      "Action": ["autoscaling:CompleteLifecycleAction", "autoscaling:RecordLifecycleActionHeartbeat"],
      "Resource": "arn:aws:autoscaling:$AWS_REGION:$ACCOUNT:autoScalingGroup:*:autoScalingGroupName/$GROUP" }
  ]
}
EOF
aws iam create-role --role-name fctl-runner --assume-role-policy-document \
  '{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Principal":{"Service":"ec2.amazonaws.com"},"Action":"sts:AssumeRole"}]}'
aws iam put-role-policy --role-name fctl-runner --policy-name fctl-runner --policy-document file://role-policy.json
aws iam create-instance-profile --instance-profile-name fctl-runner
aws iam add-role-to-instance-profile --instance-profile-name fctl-runner --role-name fctl-runner
```

Then delete `enroll-token` from your machine.

### The launch template

`user-data.sh`, which EC2 runs once, as root, as the machine first
boots:

```sh
#!/bin/sh
set -eu
printf 'GROUP=acme-runners\nHOOK=fctl-drain\n' > /etc/default/fctl-scale-in # for the watcher
md() {
  t=$(curl -sf -X PUT http://169.254.169.254/latest/api/token -H 'X-aws-ec2-metadata-token-ttl-seconds: 60') &&
    curl -sf -H "X-aws-ec2-metadata-token: $t" "http://169.254.169.254/latest/meta-data/$1"
}
for try in 1 2 3 4 5 6 7 8 9 10; do # the role can take a minute to reach a new machine
  token=$(aws secretsmanager get-secret-value --region "$(md placement/region)" --secret-id fctl/enroll-token \
    --query SecretString --output text) && break
  sleep 15
done
printf %s "$token" | fctl-enroll https://fctl.acme.dev "aws-$(md instance-id)" --capacity 3 --cpus 4 --memory 8g
```

The template asks for IMDSv2 with one hop, which keeps containers from
the metadata service on their own too, and takes the security group
without inbound rules:

```sh
cat > template.json <<EOF
{
  "ImageId": "$AMI",
  "InstanceType": "m7i.2xlarge",
  "IamInstanceProfile": { "Name": "fctl-runner" },
  "SecurityGroupIds": ["$SECURITY_GROUP"],
  "MetadataOptions": { "HttpEndpoint": "enabled", "HttpTokens": "required", "HttpPutResponseHopLimit": 1 },
  "BlockDeviceMappings": [{ "DeviceName": "/dev/sda1", "Ebs": { "VolumeSize": 200, "VolumeType": "gp3", "DeleteOnTermination": true } }],
  "UserData": "$(base64 -w0 user-data.sh)"
}
EOF
aws ec2 create-launch-template --launch-template-name fctl-runner --launch-template-data file://template.json
```

### The group, and its drain hook

In the private subnets:

```sh
aws autoscaling create-auto-scaling-group --auto-scaling-group-name "$GROUP" \
  --launch-template 'LaunchTemplateName=fctl-runner,Version=$Latest' \
  --min-size 1 --max-size 20 --desired-capacity 2 --vpc-zone-identifier "$SUBNETS"
aws autoscaling put-lifecycle-hook --auto-scaling-group-name "$GROUP" --lifecycle-hook-name fctl-drain \
  --lifecycle-transition autoscaling:EC2_INSTANCE_TERMINATING --heartbeat-timeout 7200 --default-result CONTINUE
```

The hook holds each machine Auto Scaling takes for up to two hours,
until its watcher says to go on. Turns that run longer stop at the
deadline. For longer still, have the watcher send
`aws autoscaling record-lifecycle-action-heartbeat` within each two
hours, up to 48 in all.

**Check:** within two or three minutes, two runners named
`aws-i-0…` show on the workspace's **Runners** tab as online. On a
machine, `/var/log/cloud-init-output.log` ends with the host service
started.

## 6. On Google Cloud

```sh
export PROJECT=acme-runners REGION=europe-north1 GROUP=acme-runners
gcloud config set project "$PROJECT"
```

### The image

```sh
packer build -only='googlecompute.fctl-runner' -var "fctl_version=$FCTL_VERSION" -var "gcp_project=$PROJECT" .
```

It takes about 15 minutes, and adds the image to the family
`fctl-runner`, whose newest image new machines take.

`runner/fctl-scale-in-googlecompute`, the watcher, waits for graceful
shutdown, which a managed instance group starts on the machines it
deletes, or for a Spot machine's preemption:

```sh
#!/bin/sh
# Drains this runner when Compute Engine shuts the machine down, then powers it off.
set -u
md() { curl -sf -H 'Metadata-Flavor: Google' "http://metadata.google.internal/computeMetadata/v1/instance/$1"; }
while :; do
  if [ "$(md preempted)" = TRUE ]; then fctl-drain 45; exit 0; fi # two minutes' notice, when asked for
  [ "$(md attributes/shutdown-status)" = PENDING_STOP ] && break
  sleep 5
done
fctl-drain 6600 # under the template's two hours
shutdown -h now
```

### The token, and what the machines may do

The token goes into Secret Manager, and the machines' service account
reads that secret and nothing else:

```sh
gcloud secrets create fctl-enroll-token --data-file=enroll-token
gcloud iam service-accounts create fctl-runner
gcloud secrets add-iam-policy-binding fctl-enroll-token --role=roles/secretmanager.secretAccessor \
  --member="serviceAccount:fctl-runner@$PROJECT.iam.gserviceaccount.com"
```

Then delete `enroll-token` from your machine.

### The instance template

`startup.sh`, which Compute Engine runs as root at every boot (a machine
that enrolled keeps its runner):

```sh
#!/bin/sh
set -eu
md() { curl -sf -H 'Metadata-Flavor: Google' "http://metadata.google.internal/computeMetadata/v1/$1"; }
secret="https://secretmanager.googleapis.com/v1/projects/$(md project/project-id)/secrets/fctl-enroll-token/versions/latest:access"
for try in 1 2 3 4 5 6 7 8 9 10; do
  token=$(curl -sf -H "Authorization: Bearer $(md instance/service-accounts/default/token | jq -r .access_token)" "$secret" |
    jq -r .payload.data | base64 -d) && [ -n "$token" ] && break
  sleep 15
done
printf %s "$token" | fctl-enroll https://fctl.acme.dev "$(md instance/name)" --capacity 3 --cpus 4 --memory 8g
```

```sh
gcloud compute instance-templates create fctl-runner \
  --machine-type=n2-standard-8 --image-family=fctl-runner --image-project="$PROJECT" \
  --boot-disk-size=200GB --boot-disk-type=pd-balanced --no-address \
  --service-account="fctl-runner@$PROJECT.iam.gserviceaccount.com" --scopes=cloud-platform \
  --shielded-secure-boot --shielded-vtpm --shielded-integrity-monitoring \
  --metadata=enable-oslogin=TRUE,block-project-ssh-keys=TRUE \
  --metadata-from-file=startup-script=startup.sh \
  --graceful-shutdown --graceful-shutdown-max-duration=2h
```

Graceful shutdown gives each machine the group deletes up to two hours
(six at most) before Compute Engine stops it. The machine powers itself
off once it has drained; Compute Engine bills it until then.

### The group

In a region whose subnet reaches the internet through Cloud NAT:

```sh
gcloud compute instance-groups managed create "$GROUP" --region="$REGION" --template=fctl-runner --size=2
```

**Check:** within two or three minutes, two runners named after the
group's machines (`acme-runners-x7k2`) show on the workspace's
**Runners** tab as online. On a machine, `journalctl -u
google-startup-scripts` ends with the host service started.

## 7. On Azure

```sh
export RG=acme-runners LOCATION=swedencentral GROUP=acme-runners VAULT=acme-fctl
export SUBSCRIPTION="$(az account show --query id -o tsv)"
export SUBNET_ID=/subscriptions/…/virtualNetworks/acme/subnets/runners   # with a NAT gateway
az group create -n "$RG" -l "$LOCATION"
```

### The image

Azure's scale sets start with Trusted Launch, which takes images from an
Azure Compute Gallery:

```sh
az sig create -g "$RG" --gallery-name acme_runners
az sig image-definition create -g "$RG" --gallery-name acme_runners --gallery-image-definition fctl-runner \
  --publisher Acme --offer fctl-runner --sku ubuntu-24-04 --os-type Linux --os-state Generalized \
  --hyper-v-generation V2 --features SecurityType=TrustedLaunchSupported
packer build -only='azure-arm.fctl-runner' -var "fctl_version=$FCTL_VERSION" \
  -var "azure_subscription=$SUBSCRIPTION" -var "azure_resource_group=$RG" -var "azure_location=$LOCATION" .
```

It takes about 20 minutes, and adds the version `$FCTL_VERSION` to the
image definition `fctl-runner`. The guide uses Dsv5 machines, whose disks
are SCSI: Dsv6 and later take only NVMe disks, which need an image
definition marked for them (`DiskControllerTypes=SCSI,NVMe` in
`--features`) and a scale set that asks for them.

`runner/fctl-scale-in-azure-arm`, the watcher, waits for the scale
set's terminate notification, or a Spot machine's eviction, in the
machine's scheduled events:

```sh
#!/bin/sh
# Drains this runner when Azure deletes or evicts the machine, then lets it go on.
set -u
events='http://169.254.169.254/metadata/scheduledevents?api-version=2020-07-01'
md() { curl -sf -H Metadata:true --noproxy '*' "$@"; }
me=$(md 'http://169.254.169.254/metadata/instance/compute/name?api-version=2021-02-01&format=text')
while :; do # polling keeps scheduled events on
  event=$(md "$events" | jq -c --arg me "$me" \
    'first(.Events[] | select((.EventType == "Terminate" or .EventType == "Preempt") and any(.Resources[]; . == $me)))')
  [ -n "$event" ] && break
  sleep 5
done
left=$(($(date -d "$(echo "$event" | jq -r .NotBefore)" +%s) - $(date +%s) - 60))
fctl-drain "$((left > 1 ? left : 1))"
md -X POST -d "{\"StartRequests\":[{\"EventId\":$(echo "$event" | jq .EventId)}]}" "$events"
```

### The token, and what the machines may do

The token goes into a Key Vault of the runners' own, which the scale
set's identity reads and nothing else does:

```sh
az keyvault create -g "$RG" -n "$VAULT" -l "$LOCATION" --enable-rbac-authorization true
az role assignment create --role "Key Vault Secrets Officer" \
  --assignee "$(az ad signed-in-user show --query id -o tsv)" --scope "$(az keyvault show -n "$VAULT" --query id -o tsv)"
az keyvault secret set --vault-name "$VAULT" --name enroll-token --file enroll-token
```

A vault that uses Azure's roles lets nobody at its secrets, its
subscription's owners included, until a role says so: the second
command lets you write them. It can take a minute to apply, so a
`Forbidden` from the third command goes away if you run it again.

Then delete `enroll-token` from your machine. The scale set gets its
identity, and the role on the vault, as it's created below.

### The boot script

`cloud-init.sh`, which Azure runs once, as root, as the machine first
boots:

```sh
#!/bin/sh
set -eu
md() { curl -sf -H Metadata:true --noproxy '*' "http://169.254.169.254/metadata/$1"; }
for try in 1 2 3 4 5 6 7 8 9 10; do # the role can take minutes to reach a new machine
  access=$(md 'identity/oauth2/token?api-version=2018-02-01&resource=https%3A%2F%2Fvault.azure.net' | jq -r .access_token)
  token=$(curl -sf -H "Authorization: Bearer $access" \
    'https://acme-fctl.vault.azure.net/secrets/enroll-token?api-version=7.4' | jq -r .value) && [ -n "$token" ] && break
  sleep 30
done
printf %s "$token" | fctl-enroll https://fctl.acme.dev "$(md 'instance/compute/name?api-version=2021-02-01&format=text')" \
  --capacity 3 --cpus 4 --memory 8g
```

### The scale set

On the runners' subnet, without a load balancer, as nothing connects to
the machines:

```sh
az vmss create -g "$RG" -n "$GROUP" \
  --image "/subscriptions/$SUBSCRIPTION/resourceGroups/$RG/providers/Microsoft.Compute/galleries/acme_runners/images/fctl-runner/versions/$FCTL_VERSION" \
  --security-type TrustedLaunch --vm-sku Standard_D8s_v5 --instance-count 2 \
  --os-disk-size-gb 256 --storage-sku Premium_LRS --subnet "$SUBNET_ID" --load-balancer "" \
  --admin-username acme --generate-ssh-keys \
  --custom-data cloud-init.sh --terminate-notification-time 15 \
  --assign-identity --role "Key Vault Secrets User" \
  --scope "$(az keyvault show -n "$VAULT" --query id -o tsv)"
```

The terminate notification gives each machine the scale set deletes 15
minutes, the most Azure gives, and no machine can ask for more: turns
that run longer stop at the deadline, and their agents push what they
committed.

**Check:** within three or four minutes, two runners named after the
scale set's machines (`acme-runners_4f1c2e3a`) show on the workspace's
**Runners** tab as online. On a machine, `/var/log/cloud-init-output.log`
ends with the host service started.

## 8. Check that it grows and shrinks

From any machine signed in to fctl:

```sh
fctl runners -w acme
```

Each machine is a runner there, online, with how many sessions it runs
of its capacity (`0/3`). Start a session in one of `acme`'s projects: it
starts on one of them. Then make the group smaller by one (AWS: `aws
autoscaling set-desired-capacity --desired-capacity 1`; Google Cloud:
`gcloud compute instance-groups managed resize --size=1`; Azure: `az
vmss scale --new-capacity 1`).

**Check:** within a minute or two, one of the runners says it drains.
Once its turns have ended, it leaves the Runners tab, and the cloud
deletes the machine. A session that ran there stops once its turn has
ended, and a message resumes it on another runner.

## The clouds side by side

| | AWS | Google Cloud | Azure |
|---|---|---|---|
| How long a machine it takes out may drain | 2 hours a heartbeat, 48 in all | Up to 6 hours | 15 minutes |
| Where that's set | The group's lifecycle hook | The instance template's graceful shutdown | The scale set's terminate notification |
| How the machine hears of it | `autoscaling/target-lifecycle-state` in its metadata | `shutdown-status` in its metadata | `Terminate` in its scheduled events |
| A Spot machine's notice | 2 minutes | 30 seconds; 2 minutes with `--preemption-notice-duration=120s` | 30 seconds |
| The secret store | Secrets Manager | Secret Manager | Key Vault |

## What happens to sessions when a machine goes

- **Drained.** No new session starts on the machine. Turns under way
  finish, and sessions that wait for a message stop after a minute. A
  message then resumes each on another runner, from its branch, in a
  new sandbox: the agent starts without Claude's conversation of before,
  but with its work and its transcript in fctl.
- **Past the deadline.** What still runs stops, and the agents push what
  they committed first, then it's as above. So a turn longer than the
  deadline loses what it hadn't committed.
- **Gone without a word** (a Spot reclaim without notice, a crash).
  The sessions stop ten minutes after the runner went quiet, and a
  message resumes them elsewhere, without what their agents hadn't
  pushed. fctl removes the runner after 15 minutes.

Machines that live longer keep more conversations: scale in slowly
(AWS's default cooldown, Compute Engine's scale-in controls, Azure's
autoscale rules), and keep long sessions on machines the group keeps
(its minimum size).

### Spot machines

Spot machines cost less, and the cloud takes them back with a notice of
two minutes or less. The watchers above stop their sessions at the
notice, so that the agents push their work. Keep a base of regular
machines for long sessions: on AWS, a mixed instances policy's
`OnDemandBaseCapacity`; on Google Cloud, a second group of Spot
machines beside the regular one; on Azure, a scale set with Spot
Priority Mix (`--regular-priority-count`), whose Spot machines get no
terminate notification.

### How many machines

The token's runners are a pool, and fctl says how many machines it
needs now, its `wanted`: enough for the sessions its runners run and
those that wait for it, never fewer than its runners that run one, nor
more than `--max-runners` ([Runner pools](cli.md#runner-pools)). The
enrollment token reads it, without signing in. Size the group to it
with a job that runs every minute where you run such jobs (cron on a
machine outside the group, a scheduled container), with the cloud's CLI
signed in as someone who may read the secret and resize the group:

```sh
#!/bin/sh
# fctl-size-group: sizes the group to the runners its pool wants, never below MIN.
set -eu
token=$(aws secretsmanager get-secret-value --secret-id fctl/enroll-token --query SecretString --output text)
wanted=$(FCTL_SERVER=https://fctl.acme.dev FCTL_ENROLL_TOKEN=$token fctl runners demand --json | jq .wanted)
[ "$wanted" -ge "$MIN" ] || wanted=$MIN
aws autoscaling set-desired-capacity --auto-scaling-group-name "$GROUP" --desired-capacity "$wanted"
# Google Cloud: gcloud compute instance-groups managed resize "$GROUP" --region="$REGION" --size="$wanted"
# Azure: az vmss scale --resource-group "$RESOURCE_GROUP" --name "$GROUP" --new-capacity "$wanted"
```

On Google Cloud and Azure, the job reads the token as their boot
scripts do. A token that removes offline runners gives each session to
its busiest runner that still has room, so that the others empty out;
the group still picks which machine it takes by its own rules, and the
drain lets that machine's turns end first. On AWS, `fctl runners demand
--cloudwatch --watch` puts the same numbers into CloudWatch as well,
for dashboards and alarms.

## Keeping the image current

Hosts update fctl to the version the instance runs by themselves, within
minutes, and then pull that version's sandbox image, so an older image
still works: its machines take a few minutes longer to be ready after a
release. Build the image again for each release the instance moves to,
and roll it out: AWS's instance refresh (`aws autoscaling
start-instance-refresh`), Compute Engine's rolling update (`gcloud
compute instance-groups managed rolling-action start-update`), or a new
gallery version and `az vmss update` with `az vmss update-instances`.
Each of those takes machines out as a scale-in does, through the drain.

## Each session in a microVM

fctl's `firecracker` driver runs each session in a microVM with a kernel
of its own rather than a container. It needs `/dev/kvm`, which the
clouds give machines with nested virtualization, such as EC2's C8i, M8i
and R8i, Compute Engine's N2 and Azure's Dsv6. It hasn't been tried on
the clouds' machines yet, so this guide uses Docker.

## What goes wrong

| What you see | Why | What to do |
|---|---|---|
| This enrollment token for Acme has as many runners as it may keep at once (25). | Runners of machines that went without draining still count | Raise `--max-runners` with a new token, or remove them on the Runners tab |
| This enrollment token for Acme expired on 2027-11-23 | Tokens expire | Make another, put it in the secret store |
| The host service is a systemd user unit, and fctl's systemd manager doesn't answer | The image has no lingering for `fctl` | `loginctl enable-linger fctl` in `provision.sh` |
| `AccessDeniedException`, `PERMISSION_DENIED` or `403` reading the secret | The machine's role doesn't reach the secret, or not yet | Check the role's scope; the boot script tries for a few minutes |

- **The machine runs, but no runner shows.** Its boot log
  (`/var/log/cloud-init-output.log`, or `journalctl -u
  google-startup-scripts` on Google Cloud) says why, and `sudo -u fctl
  /home/fctl/.local/bin/fctl host doctor` on it checks the rest.
- **Scale-in cuts turns off.** The watcher isn't running (`systemctl
  status fctl-scale-in`), or the hook, graceful shutdown or terminate
  notification isn't set. Without them, the cloud deletes a machine at
  once, as in a crash.
- **A runner stays on the Runners tab, offline, after its machine
  went.** The machine went without draining: fctl removes the runner
  after 15 minutes.
- **Sessions wait while runners are online.** Each runner is full
  (`fctl runners -w acme`), or drains. The workspace's queued sessions
  start as runners have room.
