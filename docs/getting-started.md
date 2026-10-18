# Getting started

This guide takes you from nothing to an agent working on your code. It
assumes an instance exists (if you are setting one up, start with
[self-hosting](self-hosting.md) and come back).

## 1. Sign in

Open your instance (for example `https://app.fctl.sh`) and sign in with the
email and password your invite set up, or with Google if your instance has it.
Access is by allowlist: if you cannot sign up, ask an admin to allow your
email.

## 2. Install the CLI

```sh
curl -fsSL https://fctl.sh/install | sh
```

The installer puts `fctl` in `~/.local/bin` after checking its SHA-256 and
sets up tab completion for your shell; `fctl self update` keeps it current.
Then sign in from the terminal; it opens your browser to approve the device:

```sh
fctl login app.fctl.sh
fctl whoami
```

## 3. Connect your Claude subscription

Sessions you start run on your own Claude Pro or Max plan. fctl never uses API
keys: it refuses them, checks that every session reports a subscription
credential, and pauses a session before it would draw on paid extra usage.

There are two ways to connect, both under **Settings → Claude** in the app:

- **Sign in with Claude** (recommended). The official Claude Code sign-in runs
  in a sandbox on one of your hosts; fctl shows you the link and relays the
  code. This kind of login lets sessions join **Remote Control**, so they show
  up in the Claude apps.
- **A long-lived token.** Run `claude setup-token` on any machine with Claude
  Code and paste the `sk-ant-oat01-…` token. It is valid for a year, but it
  cannot use Remote Control.

From the terminal:

```sh
fctl claude connect --login      # sign in with Claude through a host
fctl claude connect              # or paste a long-lived token
fctl claude status
```

## 4. Bring a machine online

Any Mac with Apple silicon (or a Linux box) can be a host. On the Mac:

1. Install Apple's container runtime from
   [github.com/apple/container/releases](https://github.com/apple/container/releases)
   (macOS 26 or later), then start it once:

   ```sh
   container system start
   ```

   On older macOS, install [Lima](https://lima-vm.io) (`brew install lima`)
   instead; on Linux, Docker.

2. Register the machine and start the host service (you need to be an admin;
   otherwise ask one for a runner token and pass `--token`):

   ```sh
   fctl host install --build-image
   ```

   This builds the sandbox image (Debian with git, gh, Node, Python, uv and
   Claude Code), writes `~/.config/fctl/host.toml`, and installs a launchd
   agent (`sh.fctl.host`) that starts at login, restarts if it stops, and
   keeps the Mac from sleeping while it is plugged in, so it can start work
   at any hour.

3. Check it:

   ```sh
   fctl host doctor
   fctl host status
   ```

The machine appears under **Admin → Runners** as online. For a Mac that runs
around the clock, keep it plugged in and turn on automatic login, so the host
starts again after a reboot or a power cut.

## 5. Create a project

In the app, choose **New project**. Link an existing GitHub repository, create
a new one, or start without a repository. Optionally add a setup script (it
runs in the clone before the agent starts, e.g. `npm ci`) and instructions for
agents. From the terminal:

```sh
fctl projects create "My app" --repo my-org/my-app
fctl projects use my-app
```

Agents can use more repositories of the same account: add them under the
project's **Repositories** tab, each read only (a private dependency, say)
or read and write, or let agents read every repository the GitHub App can
reach. From the terminal: `fctl projects repos add my-org/my-lib --write`.

## 6. Start a session

Describe the task on the floor or the project page, or:

```sh
fctl run "Add a dark mode toggle to the settings page"
```

`fctl run` follows the session live in your terminal; Ctrl-C detaches and the
agent keeps working. The agent works on a branch named
`fctl/<number>-<title>`, commits as it goes, pushes, and opens a pull request
when it is done (projects can turn that off). In the project's other
repositories it changes, it uses the same branch and opens a pull request in
each.

## 7. Follow along and steer

- **Web app** — the session page streams the transcript as the agent writes
  it: tool calls, diffs, the files changed so far. Type below to message the
  agent; it reads your message at its next step. **Interrupt** stops the
  current turn.
- **Terminal** — `fctl attach my-app#12` streams the session and lets you
  type messages; `fctl send my-app#12 "Use the existing Button component"`
  sends one from a script.
- **Claude apps** — sessions started with a full Claude login join Remote
  Control: open the session from the Claude app on your phone or at
  claude.ai/code, or run `fctl open my-app#12 --claude`.

When the agent finishes a turn, the session waits for you. It sleeps after the
project's idle timeout; **Resume** (or `fctl resume my-app#12 "…"`) picks it up
where it left off, with the same conversation and branch.

## 8. Work together

Invite people under **Admin → Access** (or `fctl admin allow ada@example.com`)
and add them to projects under **Members**. Roles:

| Role | Can |
|---|---|
| Viewer | read sessions |
| Editor | start sessions and message agents |
| Maintainer | change settings and secrets, manage members |
| Owner | everything, including deleting the project |

Share a single session with someone outside the project from its **Share**
menu. Everyone watching a session sees who else is there.
