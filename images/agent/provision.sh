#!/usr/bin/env bash
# Provisions an fctl agent sandbox: the tools agents need, Claude Code and
# fctl. Used by the Dockerfile (container sandboxes) and by `fctl host image
# build --driver lima` (the fctl-base VM). Runs as root.
#
# FCTL_VERSION picks the fctl release and FCTL_DOWNLOAD where releases come
# from (`fctl host image build` passes its own version and download_base);
# empty means the latest, from https://fctl.sh/releases. CLAUDE_VERSION
# picks Claude Code's (empty: the latest); the image workflow passes the one
# it labels the image with, so that every architecture gets the same.
set -euo pipefail

AGENT_USER="${AGENT_USER:-agent}"
NODE_MAJOR="${NODE_MAJOR:-22}"
FCTL_VERSION="${FCTL_VERSION:-}"
FCTL_DOWNLOAD="${FCTL_DOWNLOAD:-}"
CLAUDE_VERSION="${CLAUDE_VERSION:-}"
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

# fctl, for everyone on the machine: agents name and describe their session
# with `fctl edit`. A version that is not published (a development build of
# the host) gets the latest release instead. The host copies its own
# fctl-agent in at session start, so the one in the archive goes.
install_fctl() {
  curl -fsSL https://fctl.sh/install |
    env FCTL_VERSION="$1" FCTL_DOWNLOAD="$FCTL_DOWNLOAD" FCTL_INSTALL_DIR=/usr/local/bin \
      FCTL_NO_COMPLETIONS=1 sh
}
if ! install_fctl "$FCTL_VERSION"; then
  [ -n "$FCTL_VERSION" ] || exit 1
  echo "fctl $FCTL_VERSION is not published; installing the latest release" >&2
  install_fctl ""
fi
rm -f /usr/local/bin/fctl-agent-*-linux

# Claude Code, installed for the agent user (native build, ~/.local/bin/claude).
AGENT_HOME="$(getent passwd "$AGENT_USER" | cut -d: -f6)"
sudo -u "$AGENT_USER" -H env CLAUDE_VERSION="$CLAUDE_VERSION" \
  bash -c 'curl -fsSL https://claude.ai/install.sh | bash -s -- ${CLAUDE_VERSION:+"$CLAUDE_VERSION"}'
sudo -u "$AGENT_USER" -H mkdir -p "$AGENT_HOME/workspace"
grep -q '.local/bin' "$AGENT_HOME/.profile" 2>/dev/null || echo 'export PATH="$HOME/.local/bin:/opt/fctl/bin:$PATH"' >> "$AGENT_HOME/.profile"
"$AGENT_HOME/.local/bin/claude" --version
/usr/local/bin/fctl --version
