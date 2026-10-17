#!/usr/bin/env bash
# Provisions an fctl agent sandbox: the tools agents need, and Claude Code.
# Used by the Dockerfile (container sandboxes) and by `fctl host image build
# --driver lima` (the fctl-base VM). Runs as root.
set -euo pipefail

AGENT_USER="${AGENT_USER:-agent}"
NODE_MAJOR="${NODE_MAJOR:-22}"
export DEBIAN_FRONTEND=noninteractive

apt-get update
apt-get install -y --no-install-recommends \
  ca-certificates curl git gnupg less openssh-client procps sudo unzip xz-utils zip \
  build-essential pkg-config jq ripgrep fd-find tree file \
  python3 python3-venv python3-pip

# GitHub CLI
curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg -o /usr/share/keyrings/githubcli-archive-keyring.gpg
echo "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" \
  > /etc/apt/sources.list.d/github-cli.list

# Node.js
curl -fsSL "https://deb.nodesource.com/setup_${NODE_MAJOR}.x" | bash -
apt-get install -y --no-install-recommends gh nodejs
corepack enable || true

# uv (Python)
curl -LsSf https://astral.sh/uv/install.sh | env UV_INSTALL_DIR=/usr/local/bin UV_NO_MODIFY_PATH=1 sh

rm -rf /var/lib/apt/lists/*

# The agent user. The sandbox is the security boundary, so sudo needs no password.
if ! id "$AGENT_USER" >/dev/null 2>&1; then
  useradd -m -s /bin/bash -u 1000 "$AGENT_USER"
fi
echo "$AGENT_USER ALL=(ALL) NOPASSWD:ALL" > "/etc/sudoers.d/90-fctl-$AGENT_USER"
chmod 0440 "/etc/sudoers.d/90-fctl-$AGENT_USER"
mkdir -p /opt/fctl/bin
chown -R "$AGENT_USER" /opt/fctl

# Claude Code, installed for the agent user (native build, ~/.local/bin/claude).
AGENT_HOME="$(getent passwd "$AGENT_USER" | cut -d: -f6)"
sudo -u "$AGENT_USER" -H bash -c 'curl -fsSL https://claude.ai/install.sh | bash'
sudo -u "$AGENT_USER" -H mkdir -p "$AGENT_HOME/workspace"
grep -q '.local/bin' "$AGENT_HOME/.profile" 2>/dev/null || echo 'export PATH="$HOME/.local/bin:/opt/fctl/bin:$PATH"' >> "$AGENT_HOME/.profile"
"$AGENT_HOME/.local/bin/claude" --version
