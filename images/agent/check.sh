#!/bin/sh
# Checks an image built from this directory for what fctl and images built
# FROM it rely on (docs/sandbox-images.md): the agent user, sudo, the PATH and
# the tools. Runs inside the image as its user; the image workflow runs it on
# every build:
#
#   docker run --rm -i -e FCTL_VERSION=0.3.0 -e CLAUDE_VERSION=2.1.302 IMAGE sh -s < images/agent/check.sh
#
# FCTL_VERSION and CLAUDE_VERSION, when set, are the versions it must have.
set -eu

fail() {
  echo "check.sh: $*" >&2
  exit 1
}

[ "$(id -un)" = agent ] || fail "runs as $(id -un), not agent"
[ "$(id -u)" = 1000 ] || fail "agent has uid $(id -u), not 1000"
[ "$HOME" = /home/agent ] || fail "HOME is $HOME, not /home/agent"
sudo -n true 2>/dev/null || fail "agent cannot use sudo without a password"
for dir in "$HOME/.local/bin" /opt/fctl/bin; do
  case ":$PATH:" in
    *":$dir:"*) ;;
    *) fail "$dir is not on the PATH ($PATH)" ;;
  esac
done
[ -d /opt/fctl/bin ] || fail "/opt/fctl/bin is missing"
[ -d "$HOME/workspace" ] && [ -w "$HOME/workspace" ] || fail "$HOME/workspace is missing or not writable"
for tool in sh tail git gh claude fctl node npm python3 uv rg jq curl make cc; do
  command -v "$tool" >/dev/null 2>&1 || fail "$tool is not on the PATH"
done
[ "$(command -v claude)" = "$HOME/.local/bin/claude" ] || fail "claude is $(command -v claude), not ~/.local/bin/claude"

# `2.1.302 (Claude Code)` and `fctl 0.3.0`.
claude=$(claude --version)
claude=${claude%% *}
fctl=$(fctl --version)
fctl=${fctl#fctl }
[ -z "${CLAUDE_VERSION:-}" ] || [ "$claude" = "$CLAUDE_VERSION" ] ||
  fail "Claude Code is $claude, not $CLAUDE_VERSION"
[ -z "${FCTL_VERSION:-}" ] || [ "$fctl" = "$FCTL_VERSION" ] ||
  fail "fctl is $fctl, not $FCTL_VERSION"
echo "The image is as expected, with fctl $fctl and Claude Code $claude."
