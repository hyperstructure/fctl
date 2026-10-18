# Getting started

This guide takes you from nothing to an agent working on your code. It
assumes an instance exists (if you are setting one up, start with
[self-hosting](self-hosting.md) and come back).

## 1. Sign in

Open your instance (for example `https://app.fctl.sh`) and sign in with the
email and password your invite set up, or with Google if your instance has it.
Access is by allowlist: if you cannot sign up, ask an admin to allow your
email.

The app installs on your phone and computer and then opens in its own
window, from the home screen or the Dock: in Safari on an iPhone or iPad,
choose **Share → Add to Home Screen**; in Safari on a Mac, **File → Add to
Dock**; in Chrome or Edge, the install button in the address bar (on
Android, **⋮ → Add to home screen**).

To hear when a session waits for your reply, pauses on a usage limit or
fails, turn on notifications in **Settings → Notifications**, in each
browser you want them in (on an iPhone or iPad, in the app on the Home
Screen). You follow the sessions you start; the bell on a session's page
follows others.

## 2. Install the CLI

```sh
curl -fsSL https://fctl.sh/install | sh
```

The installer puts `fctl` in `~/.local/bin` after checking its SHA-256 and
sets up tab completion for your shell; `fctl self update` keeps it current.

Or install it with [Homebrew](https://brew.sh) (macOS and Linux):

```sh
brew install hyperstructure/tap/fctl
```

The formula installs the same release archive, checked against its SHA-256,
and sets up completions. Update with `brew upgrade fctl` instead of
`fctl self update`; use one install method per machine.

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

Once a session runs on it, the connection shows how much of your plan's
limits is used: the five-hour window, the week, and a model's own weekly
limit when Claude reports one, each with when it resets. You see it under
each connection in Settings → Claude, under the task box on the floor (for
the connection new sessions use), and in `fctl claude status`. Sessions
report it as they work, so it is as recent as your latest session.

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
   fctl host install
   ```

   This writes `~/.config/fctl/host.toml` and installs a launchd agent
   (`sh.fctl.host`) that starts at login, restarts if it stops, and keeps
   the Mac from sleeping while it is plugged in, so it can start work at any
   hour. The host then pulls the [sandbox image](sandbox-images.md) (Debian
   with git, gh, Python, uv, Rust, Node, TypeScript, Bun and Claude Code) in
   the background. With Lima, add `--build-image`: it prepares the VM that
   sessions are cloned from, which takes a few minutes. A host runs any
   image a project chooses; `--allow-image 'ghcr.io/acme/*'` limits that,
   and `--allow-build 'acme/*'` lets it build images from those
   repositories ([Which images a host runs](sandbox-images.md#which-images-a-host-runs)).

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
agents. When sessions need tools the default image lacks, give the project an
image of its own, or one built from a Dockerfile or devcontainer.json in its
repository, under Settings → Sandbox → Image
([Your own image](sandbox-images.md#your-own-image)). The **claude.ai
connectors** switch starts off: turn it on only if agents should use the
connectors (mail, files, docs) of whoever starts a session, without asking.
From the terminal:

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

To show the agent what you mean, attach screenshots and files: with the
paperclip, by pasting them into the message box, or by dropping them on the
page (`fctl run -a screenshot.png "…"` in the terminal). The agent sees
them in its sandbox and gets screenshots in the message itself.

`fctl run` follows the session live in your terminal; Ctrl-C detaches and the
agent keeps working. The agent works on a branch named
`fctl/<number>-<title>`, commits as it goes, pushes, and opens a pull request
when it is done (projects can turn that off). In the project's other
repositories it changes, it uses the same branch and opens a pull request in
each. Once it understands the task it renames the session to what it is
working on and describes it in a sentence or two, so the floor shows the work
rather than the first line of your task.

The agent's commits are authored by the GitHub App, with Claude as
co-author. To be a co-author of the commits in the sessions you start, set
a commit email in **Settings → Profile**: an address your GitHub account
has, such as its noreply address. Anyone who can read the repository sees it.
A project's settings decide who its commits credit: **Credit people in
commits** and **Credit Claude in commits** are both on until a maintainer
turns them off.

## 7. Follow along and steer

- **Web app** — the session page streams the transcript as the agent writes
  it: tool calls, diffs, the files changed so far. Type below to message the
  agent, with files attached if you like; it reads your message at its next
  step. **Interrupt** stops the current turn.
- **Terminal** — `fctl attach my-app#12` streams the session and lets you
  type messages; `fctl send my-app#12 "Use the existing Button component"`
  sends one from a script.
- **Claude apps** — sessions started with a full Claude login join Remote
  Control. **Open in Claude** on the session page opens the session in
  Claude's desktop or mobile app, and **Open on claude.ai** in its menu opens
  it in a browser. From a terminal, `fctl open my-app#12 --claude` opens the
  desktop app, or claude.ai when it is not installed.

If the agent's work stops coming through, because the session's runner went
away or can't reach the agent in its sandbox, the session page says so and
since when. The agent may well be working meanwhile: its work shows up once
the runner gets through, and messages you send wait for it. After ten
minutes away the session shows as stopped, and a message resumes it on
another runner. If the runner comes back with the agent still running
before the session runs anywhere else, the session carries on there.

When the agent finishes a turn, the session waits for you. After the
project's idle timeout (ten minutes unless its settings say otherwise) it
stops, so that its machine takes other work. Work the agent left running
in the background, like a review by a subagent or a long build, keeps the
session up for up to an hour after the turn; the agent picks up its
result in a turn of its own. A message picks it up where it
left off, on the same machine, with the same conversation and branch: type
it on the session page or send it with `fctl send`. A session that failed
needs its owner to **Resume** it (or `fctl resume my-app#12 "…"`).

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
