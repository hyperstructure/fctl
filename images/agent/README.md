# Agent image

The default sandbox for fctl sessions: Debian bookworm with git, the GitHub
CLI, Node.js 22, Python 3 with uv, build tools, ripgrep, Claude Code
(installed for the `agent` user, who has passwordless sudo: the sandbox is
the boundary) and fctl (in `/usr/local/bin`).
[Sandbox images](../../docs/sandbox-images.md) describes it for the people
who run it or build images from it.

- The Image workflow (`.github/workflows/image.yml`) publishes it as
  `ghcr.io/hyperstructure/fctl-agent` for arm64 and amd64: a release's
  image is built from its tag, with that release of fctl and the newest
  Claude Code (`CLAUDE_VERSION`), and the latest release's is rebuilt when
  Claude Code has a newer release. Hosts pull the tag of their own version.
- Images built `FROM` it rely on its layout: the `agent` user (uid 1000,
  passwordless sudo), `~/.local/bin` and `/opt/fctl/bin` first on the PATH,
  and the tools. `check.sh` checks every build for it; keep it, or change
  it together with the docs.
- `fctl host image build` builds it on a host instead, under the host's
  image name, with the host's driver (Apple `container build` or `docker
  build`), or provisions the `fctl-base` Lima VM with the same
  `provision.sh`. It installs the host's own version of fctl
  (`FCTL_VERSION`; the latest release when that one is not published).
- Agents use fctl to name and describe their session: `fctl edit --title …
  --description …` reaches fctl-agent over the session socket and needs no
  sign-in. At each start fctl-agent writes how to do that to `~/AGENTS.md`,
  beside the checkout in `~/workspace`, and imports it from Claude's user
  memory (`~/.claude/CLAUDE.md`), so it applies in every repository.
