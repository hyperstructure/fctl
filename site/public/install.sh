#!/bin/sh
# Installs fctl: curl -fsSL https://fctl.sh/install | sh
#
# Environment:
#   FCTL_VERSION      a version to install (default: the latest)
#   FCTL_INSTALL_DIR  where to put the binaries (default: ~/.local/bin)
#   FCTL_DOWNLOAD     release base URL (default: https://fctl.sh/releases)
#
# It downloads one archive for your platform, checks its SHA-256, and puts
# `fctl` (plus the Linux agent its sandboxes run) in the install directory.
# Nothing runs as root and nothing outside that directory changes.

set -eu

BASE="${FCTL_DOWNLOAD:-https://fctl.sh/releases}"
DIR="${FCTL_INSTALL_DIR:-$HOME/.local/bin}"

say() { printf '%s\n' "$*"; }
err() { printf 'fctl install: %s\n' "$*" >&2; exit 1; }

fetch() {
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL --proto '=https' --tlsv1.2 --retry 3 "$1"
  elif command -v wget >/dev/null 2>&1; then
    wget -qO- --https-only "$1"
  else
    err "curl or wget is needed"
  fi
}

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | cut -d' ' -f1
  else
    err "sha256sum or shasum is needed to verify the download"
  fi
}

case "$(uname -s)" in
  Darwin) os=apple-darwin ;;
  Linux) os=unknown-linux-musl ;;
  *) err "unsupported system: $(uname -s) (fctl runs on macOS and Linux)" ;;
esac
case "$(uname -m)" in
  arm64 | aarch64) arch=aarch64 ;;
  x86_64 | amd64) arch=x86_64 ;;
  *) err "unsupported architecture: $(uname -m)" ;;
esac
# A shell under Rosetta reports x86_64 on Apple silicon: install the native build.
if [ "$os" = apple-darwin ] && [ "$arch" = x86_64 ] &&
  [ "$(sysctl -n sysctl.proc_translated 2>/dev/null || echo 0)" = 1 ]; then
  arch=aarch64
fi
target="$arch-$os"

version="${FCTL_VERSION:-}"
if [ -z "$version" ]; then
  version="$(fetch "$BASE/latest" | tr -d '[:space:]')" || err "could not look up the latest version"
fi
version="${version#v}"
case "$version" in
  [0-9]*.[0-9]*.[0-9]*) ;;
  *) err "unexpected version: $version" ;;
esac

tmp="$(mktemp -d 2>/dev/null || mktemp -d -t fctl)"
trap 'rm -rf "$tmp"' EXIT INT TERM

archive="fctl-$target.tar.gz"
url="$BASE/v$version/$archive"
say "Downloading fctl $version for $target"
fetch "$url" >"$tmp/$archive" || err "download failed: $url"
expected="$(fetch "$url.sha256" | cut -d' ' -f1)" || err "checksum download failed"
actual="$(sha256 "$tmp/$archive")"
if [ -z "$expected" ] || [ "$expected" != "$actual" ]; then
  err "checksum mismatch for $archive"
fi

tar -xzf "$tmp/$archive" -C "$tmp"
mkdir -p "$DIR"
for f in fctl "fctl-agent-$arch-linux"; do
  [ -f "$tmp/$f" ] || err "$f is missing from the archive"
  chmod 755 "$tmp/$f"
  mv -f "$tmp/$f" "$DIR/$f"
done

say "Installed $DIR/fctl ($("$DIR/fctl" --version 2>/dev/null || echo "fctl $version"))"
case ":$PATH:" in
  *":$DIR:"*) ;;
  *)
    say ""
    say "$DIR is not on your PATH. Add it, e.g.:"
    case "${SHELL:-}" in
      */zsh) say "  echo 'export PATH=\"$DIR:\$PATH\"' >> ~/.zshrc && exec zsh" ;;
      */fish) say "  fish_add_path $DIR" ;;
      *) say "  echo 'export PATH=\"$DIR:\$PATH\"' >> ~/.profile" ;;
    esac
    ;;
esac
say ""
say "Next:"
say "  fctl login <your instance>    # e.g. fctl login app.fctl.sh"
say "  fctl host install            # make this machine run agents"
