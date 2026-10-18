#!/usr/bin/env bash
# shellcheck disable=SC2016 # single-quoted scripts expand in the agent's shell
# Provisions an fctl agent sandbox: the tools agents need, Claude Code and
# fctl. Used by the Dockerfile (container sandboxes) and by `fctl host image
# build --driver lima` (the fctl-base VM). Runs as root.
#
# FCTL_VERSION picks the fctl release and FCTL_DOWNLOAD where releases come
# from (`fctl host image build` passes its own version and download_base);
# empty means the latest, from https://fctl.sh/releases.
set -euo pipefail

AGENT_USER="${AGENT_USER:-agent}"
# Toolchain versions. Rust is the release this repository builds with
# (rust-toolchain.toml; a test in crates/fctl-host/src/image.rs keeps the two
# in step), so sessions working on fctl have nothing to download.
NODE_MAJOR="${NODE_MAJOR:-22}"
TYPESCRIPT_VERSION="${TYPESCRIPT_VERSION:-7.0.2}"
BUN_VERSION="${BUN_VERSION:-1.4.3}"
UV_VERSION="${UV_VERSION:-0.13.1}"
PYTHON_VERSION="${PYTHON_VERSION:-3.15.0}"
RUST_VERSION="${RUST_VERSION:-1.99.0}"
FCTL_VERSION="${FCTL_VERSION:-}"
FCTL_DOWNLOAD="${FCTL_DOWNLOAD:-}"
export DEBIAN_FRONTEND=noninteractive

apt-get update
apt-get install -y --no-install-recommends \
  ca-certificates curl wget git gnupg less openssh-client procps psmisc lsof sudo \
  rsync unzip xz-utils zip zstd \
  build-essential cmake pkg-config libssl-dev python3 \
  jq ripgrep fd-find tree file shellcheck sqlite3
# Debian calls fd `fdfind`.
ln -sf /usr/bin/fdfind /usr/local/bin/fd

# GitHub CLI
curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg -o /usr/share/keyrings/githubcli-archive-keyring.gpg
echo "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" \
  > /etc/apt/sources.list.d/github-cli.list

# Node.js and TypeScript. npm leaves out its funding and audit notices and
# its update check, which only lengthen agents' output.
curl -fsSL "https://deb.nodesource.com/setup_${NODE_MAJOR}.x" | bash -
apt-get install -y --no-install-recommends gh nodejs
corepack enable || true
npm config set --location=global fund=false audit=false update-notifier=false
npm install -g "typescript@$TYPESCRIPT_VERSION"
npm cache clean --force

# uv, which also installs the agent's Python below
curl -LsSf "https://astral.sh/uv/$UV_VERSION/install.sh" | env UV_INSTALL_DIR=/usr/local/bin UV_NO_MODIFY_PATH=1 sh

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
sudo -u "$AGENT_USER" -H bash -c 'curl -fsSL https://claude.ai/install.sh | bash'
sudo -u "$AGENT_USER" -H mkdir -p "$AGENT_HOME/workspace"

# The agent's PATH: Claude Code and Python in ~/.local/bin, then Rust's and
# Bun's tools and fctl's supervisor. The Dockerfile sets the same; login
# shells (setup scripts, the Lima driver's) get it from here.
cat > /etc/profile.d/fctl.sh <<'EOF'
PATH="$HOME/.local/bin:$HOME/.cargo/bin:$HOME/.bun/bin:/opt/fctl/bin:$PATH"
EOF

# Runs a script as the agent user in a login shell; it gets the arguments
# after the script as $1….
as_agent() {
  local script="$1"
  shift
  sudo -u "$AGENT_USER" -H bash -lc "set -euo pipefail; $script" bash "$@"
}

# Toolchains installed for the agent user, who can then add Python versions,
# Rust toolchains and global packages without sudo: CPython as python3 and
# python (uv's build), Rust with the components and targets that
# rust-toolchain.toml lists, and Bun.
as_agent 'uv python install --default --preview-features python-install-default "$1"' "$PYTHON_VERSION"
as_agent 'curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs |
  sh -s -- -y --no-modify-path --profile minimal --default-toolchain "$1" \
    --component rustfmt,clippy --target wasm32-unknown-unknown' "$RUST_VERSION"
as_agent 'curl -fsSL https://bun.sh/install | bash -s "bun-v$1"' "$BUN_VERSION"
as_agent 'ln -sf bun "$HOME/.bun/bin/bunx"'

# What sessions find on the PATH.
as_agent 'claude --version; fctl --version; python3 --version; uv --version
  rustc --version; cargo --version; node --version; tsc --version; bun --version'
