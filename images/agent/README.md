# Agent image

The default sandbox for fctl sessions: Debian bookworm with git, the GitHub
CLI, Node.js 22, Python 3 with uv, build tools, ripgrep, and Claude Code
(installed for the `agent` user, who has passwordless sudo: the sandbox is
the boundary).

- `fctl host image build` builds it locally as `fctl-agent:latest` with the
  host's driver (Apple `container build` or `docker build`), or provisions
  the `fctl-base` Lima VM with the same `provision.sh`.
- Projects can use any other image (Settings → Sandbox image). It needs
  `sh`, `tail`, git and Claude Code on the PATH; fctl copies its supervisor
  into `/opt/fctl/bin` at session start.
