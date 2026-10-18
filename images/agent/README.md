# Agent image

The default sandbox for fctl sessions: Debian bookworm with git, the GitHub
CLI, Node.js 22, Python 3 with uv, build tools, ripgrep, Claude Code
(installed for the `agent` user, who has passwordless sudo: the sandbox is
the boundary) and fctl (in `/usr/local/bin`).

- `fctl host image build` builds it locally as `fctl-agent:latest` with the
  host's driver (Apple `container build` or `docker build`), or provisions
  the `fctl-base` Lima VM with the same `provision.sh`. It installs the
  host's own version of fctl (`FCTL_VERSION`; the latest release when that
  one is not published).
- Agents use fctl to name and describe their session: `fctl edit --title …
  --description …` reaches fctl-agent over the session socket and needs no
  sign-in. At each start fctl-agent writes how to do that to `~/AGENTS.md`,
  beside the checkout in `~/workspace`, and imports it from Claude's user
  memory (`~/.claude/CLAUDE.md`), so it applies in every repository.
- Projects can use any other image (Settings → Sandbox image). It needs
  `sh`, `tail`, git and Claude Code on the PATH; fctl copies its supervisor
  into `/opt/fctl/bin` at session start. With fctl on the PATH as well,
  agents name their sessions; without it, sessions keep the title they
  start with.
