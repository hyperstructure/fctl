# Getting started

This guide takes you from nothing to an agent working on your code. It
assumes an instance exists (if you are setting one up, start with
[self-hosting](self-hosting.md) and come back).

## 1. Sign in

Open your instance (for example `https://app.fctl.sh`) and sign in with the
email and password your invite set up, or with Google if your instance has it.
Access is by allowlist: if you cannot sign up, ask an admin to allow your
email.

Have more than one address, say a private one and your company's? An
instance admin invites each on its own. Open the second invitation while you
are signed in and choose **Connect it to my account**: both addresses then
sign in to the same account, each with its own commit email (**Settings →
Email addresses**), and the workspaces you join as one of them see only
that one.

Add a passkey in **Settings → Passkeys**, and your fingerprint, face or
screen lock signs you in from then on, without a password: the browser
offers it in the email field. Turn on two-factor sign-in in **Settings →
Two-factor sign-in**, so that a password alone can't sign anyone in: scan
the QR code with an authenticator app (1Password, Google Authenticator, the
passwords app on your phone) and keep the recovery codes it gives you. If
your instance requires two-factor, it asks you to add a passkey or set up
an app when you first sign in.

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

## 3. Create or join a workspace

Projects, the machines that run their agents and the GitHub accounts they
work in belong to workspaces: one for your company's repositories, one for
your own, one for a side project with friends. An invitation to a workspace
shows on your home page, where you join it; or choose **New workspace** in
the sidebar (`fctl workspaces create "Side project"`) and be its primary
admin. Each workspace has its own people, admins, runners, GitHub accounts
and audit log, and can require two-factor sign-in of its members. You can be
in up to five. Until a new workspace has them, its page asks its admins for
people, a runner, a GitHub account and a project; in a workspace for one,
**Skip for now** leaves out the people. Drag workspaces, and the projects
in each, into the order you want in the sidebar (Alt or Option with ↑ or
↓ moves the focused one): the order follows you to your other devices and
to `fctl workspaces` and `fctl projects list`.

## 4. Connect your Claude subscription

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
each connection in Settings → Claude, under the task box on the home page
(for the connection new sessions use), and in `fctl claude status`. Sessions
report it as they work, so it is as recent as your latest session.

With more than one connection, say your own plan and a seat your company
pays for, choose which one each workspace's sessions run on in **Settings
→ Workspaces** (`fctl workspaces claude <connection> -w acme`, with the
name or id `fctl claude status` shows).

## 5. Bring a machine online

A workspace's sessions run only on its runners: until one is online, they
wait in the queue, and the app says so under the task box. Its **Runners**
tab shows how to add a runner, under the ones it has. A runner its admins
add runs everyone's sessions; one a member adds runs only theirs.

Any Mac with Apple silicon (or a Linux box) can be a host. On the Mac:

1. Install Apple's container runtime from
   [github.com/apple/container/releases](https://github.com/apple/container/releases)
   (macOS 26 or later), then start it once:

   ```sh
   container system start
   ```

   On older macOS, install [Lima](https://lima-vm.io) (`brew install lima`)
   instead; on Linux, Docker.

2. Register the machine for a workspace you are in, and start the host
   service (or pass `--token` with a runner token from the workspace's
   **Runners** tab). As one of its admins you register it for everyone's
   sessions there (`--personal`: only yours); as a member, for yours:

   ```sh
   fctl host install --workspace acme
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

The machine appears under the workspace's **Runners** tab as online, and
the workspace's sessions start on it. It is yours: add it to your other
workspaces from their Runners tab, or with `fctl workspaces add-runner -w
side-project` on the machine, and it takes the sessions of all of them.
Elsewhere, that command takes the runner's name or the id `fctl host
status` shows. For a Mac that runs around the clock, keep it plugged in
and turn on automatic login, so the host starts again after a reboot or a
power cut.

A runner sees the code, secrets and Claude token of each session it runs.
To keep yours on machines you control, turn on **Only on my runners** in
the workspace's **Settings** (`fctl workspaces own-runners on -w acme`):
your sessions there then run only on runners you registered, and wait
while none of those is online. A Claude sign-in through a host runs on one
of your runners when one is online.

Keep it on the version of fctl the app runs. While a runner of yours is
online with an older one, your home page says so, and **How to update**
shows what to run on it: `fctl self update`, which restarts the host
service while its sessions carry on (or `brew upgrade fctl`, then `fctl
host restart`, where Homebrew installed fctl). **Skip this version** hides
the notice in that browser until the app runs a newer version.

## 6. Create a project

In the app, choose **New project** (or the **+** beside a workspace in the
sidebar); a workspace's admins create its projects, and add the people who
work in them. Link an existing repository of a GitHub account the workspace
works in, create a new one, or start without a repository. A workspace
works in an account once the instance's GitHub App is installed there and
one of its admins connects it under the workspace's **GitHub** tab, by
signing in to GitHub, which connects what that person may give the
workspace. Optionally add a setup script (it
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

## 7. Start a session

Describe the task on the home page or the project page, or:

```sh
fctl run "Add a dark mode toggle to the settings page"
```

On a computer, the home page opens with the cursor in its task box. From
anywhere in the app, ⌘K and then Enter take you there (Ctrl+K and Enter on
Linux and Windows), and ⌘Enter (Ctrl+Enter) starts the session. ⌘K opens
the search that jumps to sessions, projects and workspaces, and ⌘P and ⌘⇧P
open it too, as in code editors.

The two lists by the Start button choose the model the session runs on
(Opus, Sonnet, Haiku or Fable, each the latest of its family) and its
effort level. They start on what you chose when you last started a session
in the project, else on the project's settings (its **Settings** tab sets
them for everyone). On a phone, Start is an arrow (↑), and the lists show
the model's name and the effort level's symbol, the one Claude Code shows
for it: ○ low, ◐ medium, ● high, ◉ extra high, ◈ max. In the terminal:
`fctl run --model haiku --effort low "…"`.

To show the agent what you mean, attach screenshots and files: with the
paperclip, by pasting them into the message box, or by dropping them on the
page (`fctl run -a screenshot.png "…"` in the terminal). The agent sees
them in its sandbox and gets screenshots in the message itself.

A task you write on the home page stays in your browser, with its project,
model, effort level and files, until a session starts with it: leave the
page, and it is there when you come back.

`fctl run` follows the session live in your terminal; Ctrl-C detaches and the
agent keeps working. The agent works on a branch named
`fctl/<number>-<title>`, commits as it goes, pushes, and opens a pull request
when it is done (projects can turn that off). In the project's other
repositories it changes, it uses the same branch and opens a pull request in
each. The session page and the session lists name every pull request the
session opens, with its repository (`fctl#120`, `design#14`), as does
`fctl show`. The session page's Changes section shows what the branch
changed since it left the base, the first ten files of each repository and
the rest a click away; once its pull request is merged, Changes
says so, and fctl no longer pushes the branch. Once it understands the task it renames the session to what it
is working on and describes it in a sentence or two, so the home page shows
the work rather than the first line of your task.

The agent's commits are authored by the GitHub App, with Claude as
co-author. To be a co-author of the commits in the sessions you start, set
a commit email in **Settings → Email addresses**, for the address you are
in the workspace as: an address your GitHub account has, such as its
noreply address. Anyone who can read the repository sees it.
A project's settings decide who its commits credit: **Credit people in
commits** and **Credit Claude in commits** are both on until a maintainer
turns them off.

## 8. Follow along and steer

- **Web app** — the session page streams the transcript as the agent writes
  it: tool calls, diffs, and the files changed, commits and pull requests
  so far, seconds after the agent makes them. The tool calls and thoughts
  between two things the agent says fold into one line that says what they
  did, like "Ran 6 commands, read 2 files", and opens into them; while the
  agent works, its newest step shows under that line. Where the agent delegated to
  a subagent, its line opens into what the subagent was asked, its steps
  as it takes them, in the background too, and its report. A click on an
  image or a screenshot shows it at full size, where the arrow keys, or a
  swipe on a phone, step through the session's others, and on a phone a
  pinch or a double tap zooms in on it. On a phone, tap
  the session's title for its details: its pull requests, the files
  changed, its branch and who else is watching. Type below to message the
  agent, with files attached if you like; it reads your message at its
  next step. **Interrupt** stops the current turn.
- **Terminal** — `fctl attach my-app#12` streams the session and lets you
  type messages; `fctl send my-app#12 "Use the existing Button component"`
  sends one from a script.
- **Claude apps** — a session started with a full Claude login joins Remote
  Control, in the Claude apps of the person whose subscription it runs on:
  who started it, or who resumed it. Only they can open it there, so only
  they see **Open in Claude** on the session page, which opens it in
  Claude's desktop or mobile app, and **Open on claude.ai** in its menu,
  which opens it in a browser. From a terminal,
  `fctl open my-app#12 --claude` opens the desktop app, or claude.ai when it
  is not installed.

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

## 9. Work together

A workspace's admins invite people from its **People** tab (`fctl
workspaces invite grace@example.com`), by an address they sign in with;
they join it from their home page. An address the instance doesn't admit yet
needs an instance admin's invitation first (**Admin → People → Invite**, or
`fctl admin invite grace@example.com -w side-project`), which can invite it
to the admin's workspaces at the same time. One the allowlist admits but
nobody signed up with yet can be invited right away: its owner joins when
they sign up.

In a workspace, admins own every project, and members work in the projects
they are members of (a project's **Members**) and in every project with the
workspace's base role, which its admins choose in its settings:

| Role | Can |
|---|---|
| Viewer | read sessions |
| Editor | start sessions and message agents |
| Maintainer | change settings and secrets, manage members |
| Owner | everything, including deleting the project |

Share a single session with someone else in the workspace from its
**Share** menu. Everyone watching a session sees who else is there.

A workspace has one primary admin, whom no other admin can demote or
remove; they hand the role to another admin under its **Settings**, as the
primary instance owner does under **Admin → People**.
