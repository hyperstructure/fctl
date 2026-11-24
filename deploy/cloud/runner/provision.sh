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
# What retires an idle machine and what sizes its group, where the cloud has them.
for script in retire size-group; do
  [ ! -f "/tmp/runner/fctl-$script-$CLOUD" ] || install -m 0755 "/tmp/runner/fctl-$script-$CLOUD" "/usr/local/sbin/fctl-$script"
done
install -m 0644 /tmp/runner/fctl-firewall.service /tmp/runner/fctl-scale-in.service \
  /tmp/runner/fctl-size-group.service /tmp/runner/fctl-size-group.timer /etc/systemd/system/
systemctl enable fctl-firewall.service fctl-scale-in.service fctl-size-group.timer

# The host's user, in the docker group, whose systemd manager starts at boot.
useradd --create-home --shell /bin/bash fctl
usermod -aG docker fctl
loginctl enable-linger fctl

# fctl, in a directory the host's user writes to, where the host updates it.
runuser -u fctl -- sh -c 'cd && curl -fsSL https://fctl.sh/install | FCTL_NO_COMPLETIONS=1 sh'

# The sandbox image of that version, so that a new machine's first session doesn't wait for it.
docker pull "ghcr.io/hyperstructure/sandbox:$FCTL_VERSION"
