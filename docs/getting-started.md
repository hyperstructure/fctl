# Getting started

This guide takes you from nothing to an agent working on your code. It
assumes an instance exists: [app.fctl.sh](https://app.fctl.sh), the managed
control plane that hyperstructure runs, or one you host yourself (if you
are setting one up, start with [self-hosting](self-hosting.md) and come
back).

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

Add a passkey in **Settings → Security**, and your fingerprint, face or
screen lock signs you in from then on, without a password: the browser
offers it in the email field. Turn on **Two-factor sign-in** on the same
page, so that a password alone can't sign anyone in: scan the QR code with
an authenticator app (1Password, Google Authenticator, the passwords app on
your phone) and keep the recovery codes it gives you.
Click **Authenticator app** there to name it after the app that keeps it,
such as 1Password. If
your instance requires two-factor, it asks you to add a passkey or set up
an app when you first sign in. Where the instance asks for it, sensitive
actions, such as deleting a project or making an access token, take your
passkey or a code too, at most once every 15 minutes in each browser.

The app installs on your phone and computer and then opens in its own
window, from the home screen or the Dock: in Safari on an iPhone or iPad,
choose **Share → Add to Home Screen**; in Safari on a Mac, **File → Add to
Dock**; in Chrome or Edge, the install button in the address bar (on
Android, **⋮ → Add to home screen**). On an iPhone or iPad, the app on the
Home Screen keeps no history of the pages you open, so a swipe from the
edge of the screen doesn't go back or forward: you move about with the
sidebar and links.

**Settings → App log** keeps what the app did in that browser lately: how
it started, when it went to the background and came back, and what went
wrong. It stays in the browser; signing out, or a sign-in that ended, clears
it. When the app misbehaves (on an iPhone, coming back from the background
black, say), copy it into a message about it. Where the app comes back
showing nothing while it still runs, it loads its page again by itself, at
most once a minute.

To hear when a session waits for your reply, pauses on a usage limit or
fails, turn on notifications in **Settings → Notifications**, in each
browser you want them in (on an iPhone or iPad, in the app on the Home
Screen). You follow the sessions you start; the bell on a session's page
follows others. An autopilot's sessions notify you only when they ask you
something: it looks after the rest itself.

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
shows on your Agent Floor, where you join it; or choose **New workspace** in
the sidebar (`fctl workspaces create "Side project"`) and be its primary
admin. Each workspace has its own people, admins, runners, GitHub accounts
and audit log, and can require two-factor sign-in of its members. You can be
in up to five, and the primary instance owner in any number. Until a new
workspace has them, its page asks its admins for people, a runner, a GitHub
account and a project; in a workspace for one, **Skip for now** leaves out
the people. Drag workspaces, and the projects
in each, into the order you want in the sidebar (Alt or Option with ↑ or
↓ moves the focused one): the order follows you to your other devices and
to `fctl workspaces` and `fctl projects list`.

## 4. Connect your Claude subscription

Sessions you start run on your own Claude Pro or Max plan. fctl never uses API
keys: it refuses them, checks that every session reports a subscription
credential, and pauses a session before it would draw on paid extra usage.

Settings → Claude offers two kinds of connection, each with what it gives
and lacks:

- **A long-lived token** (recommended). Run `claude setup-token` on any
  machine with Claude Code and paste the `sk-ant-oat01-…` token. It lasts a
  year, so a factory that runs around the clock keeps working without anyone
  signing in again. Its sessions don't join **Remote Control** and get no
  claude.ai connectors. Like every Claude token, it is in its sessions'
  sandboxes, where the agent can read it, so an agent talked into handing it
  over gives away a year of access rather than a full login's few hours:
  use it where you trust everyone who can message your agents.
- **Sign in with Claude**. The official Claude Code sign-in runs in a
  sandbox on one of your runners; fctl shows you the link and relays the
  code. Sessions on it join **Remote Control**, so they show up in the
  Claude apps, and can use your claude.ai connectors in projects that turn
  them on. Claude ends such a login after a while: its sessions stop, and
  Settings → Claude says it needs reconnecting, until you sign in again or
  connect a long-lived token.

A long-lived token can also stay **on your runners**, never given to fctl.
On each of your runners ([step 5](#5-bring-a-machine-online)), make a token
and keep it there:

```sh
claude setup-token             # there, or on any machine with Claude Code
fctl host claude set-token     # paste it; kept in ~/.config/fctl/claude-token, mode 0600
fctl host claude status        # whether this runner keeps one, and whether fctl knows
```

Then choose **Long-lived token → My runners keep it** in Settings → Claude,
which shows each of your runners as it reports whether it keeps the token.
Your sessions on it run only on your runners that keep it, in every
workspace, and wait while none of them is online.

From the terminal:

```sh
fctl claude connect              # paste a long-lived token
fctl claude connect --on-runners # or use the token your runners keep
fctl claude connect --login      # or sign in with Claude, on one of your runners
fctl claude status
```

Once a session runs on it, the connection shows how much of your plan's
limits is used: the five-hour window (Ultra plans have none), the week, and
a model's own weekly limit when Claude reports one, each with when it resets. You see it under
each connection in Settings → Claude, under the task box on the Agent Floor
(for the connection new sessions use), and in `fctl claude status`. Sessions
report it as they work, so it is as recent as your latest session. From
80 % of a limit the Agent Floor says how much of it is used, in red from
90 %; sessions pause when one is used up, until it resets.

With more than one connection, say your own plan and a seat your company
pays for, choose which one each workspace's sessions run on in **Settings
→ Workspaces** (`fctl workspaces claude <connection> -w acme`, with the
name or id `fctl claude status` shows).

## 5. Bring a machine online

A workspace's sessions run only on its runners: until one is online, they
wait in the queue, and the app says so under the task box. Its **Runners**
tab shows how to add a runner, under the ones it has. A runner its admins
add runs everyone's sessions; one a member adds runs only theirs.

A runner is a machine with the `fctl host` service on it: `fctl host
install` registers the machine and starts the service, which connects out
to fctl, takes the runner's sessions, makes a sandbox for each and starts
its agent there. One machine is one runner: it runs several sessions at
once (4 unless `--capacity` says otherwise), all in sandboxes from the same
driver, and can run sessions for several workspaces. You look after it on
the machine itself with `fctl host` (`status`, `drain`, `uninstall`), and
from anywhere on the Runners tab or with `fctl runners`.

Any Mac with Apple silicon (or a Linux box) can be a runner. On the Mac:

1. Install Apple's container runtime from
   [github.com/apple/container/releases](https://github.com/apple/container/releases)
   (macOS 26 or later), then start it once:

   ```sh
   container system start
   ```

   It stops when the Mac restarts; the host service starts it again.

   On older macOS, install [Lima](https://lima-vm.io) (`brew install lima`)
   instead.

   On Linux, install Docker Engine with
   [Docker's guide for your distribution](https://docs.docker.com/engine/install/),
   and start it, now and whenever the machine starts:

   ```sh
   sudo systemctl enable --now docker
   ```

   Docker lets in only root and the members of its `docker` group.
   `fctl host install` offers to add you (`sudo usermod -aG docker
   $USER`), and fctl takes the group by itself, so you don't log out and
   back in. For a microVM of its own for each session instead, use KVM and
   `passt` ([Firecracker](cli.md#hosts): `sudo usermod -aG kvm $USER`,
   `sudo apt install passt e2fsprogs`, and `--driver firecracker` below).

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
   hour; on Linux, a systemd user unit, `fctl-host.service`. The host then pulls the [sandbox image](sandbox-images.md) (Debian
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
and turn on automatic login, so the host service starts again after a
reboot or a power cut.

A runner sees the code, secrets and Claude token of each session it runs.
To keep yours on machines you control, turn on **Only on my runners** in
the workspace's **Settings** (`fctl workspaces own-runners on -w acme`):
your sessions there then run only on runners you registered, and wait
while none of those is online. A Claude sign-in (`fctl claude connect
--login`) runs on one of your runners when one is online. To keep your
Claude token away from fctl too, keep it on your runners
([step 4](#4-connect-your-claude-subscription)): your sessions on it then
run only on your runners that keep it, in every workspace.

The host keeps itself on the version of fctl the app runs: when the app
runs a newer one, the host downloads it, checks it, and restarts on it once
no session is being started or stopped there, while its sessions carry on.
To update it yourself instead, turn that off with `fctl host auto-update
off` (or install with `--auto-update off`). Where Homebrew installed fctl,
Homebrew updates it: `brew upgrade fctl`, then `fctl host restart`. While a
runner of yours that doesn't update itself is online with an older fctl
than the app, your Agent Floor says so, and **How to update** shows what to
run on it: `fctl self update`, which restarts the host service while its
sessions carry on. **Skip this version** hides the notice in that browser
until the app runs a newer version.

### Many machines and autoscaling

Machines that autoscaling starts and stops, or that you set up without
signing in on them, enroll themselves with one of the workspace's
**enrollment tokens**. Its admins make one under **Enrollment tokens** on
its Runners tab (or with `fctl runners enrollment-tokens create`): a name,
how long it works, how many runners it may enroll at once, whether each
waits for an admin's approval, and whether runners that stay offline are
removed. The token is shown once. On each machine, from its boot script
(cloud-init's user data, a launch template) or a shell:

```sh
curl -fsSL https://fctl.sh/install | sh
FCTL_SERVER=https://app.fctl.sh FCTL_ENROLL_TOKEN=fctl_et_… fctl host install
```

The machine becomes a runner of the workspace, with a token of its own:
the enrollment token is for enrolling, nothing else. The runner belongs to
the workspace rather than to a person, runs everyone's sessions there and
no other workspace's, and stays when people leave. Its admins rename,
drain, disable and remove it on the Runners tab. On Linux, where nobody
logs in, run `loginctl enable-linger` for the user the host runs as first,
so that its systemd user service can start.

With approval, a new runner waits on the Runners tab, with what it said of
itself and where it enrolled from, and the workspace's admins get a
notification. It runs nothing until one of them approves it, and then
takes sessions at once; `fctl host status` on the machine shows its id to
compare. Run again, `fctl host install` keeps the runner the machine has,
and `fctl host uninstall` removes it; a token can also have runners that
stay offline removed after a while, for machines that go without
uninstalling. A runner of the workspace keeps no one's Claude token: people
who run their sessions only on their own runners, or keep their token on
them, don't run sessions on it.

To enroll machines of your own instead, for your sessions alone, make a
token for **Only my sessions** (any member can; a member's tokens are
always for their own sessions, and admins choose), or
`fctl runners enrollment-tokens create "My cluster" -w acme --personal
--remove-offline-after 5m`. The machines it enrolls are your runners: they
run only your sessions in that workspace and no other's, so they count for
**Only on my runners** and for the Claude token you keep on your runners,
and you manage them as your other runners. With approval, they wait for
you, not the workspace's admins. If you keep your Claude token on your
runners, give each machine the token from `claude setup-token` as a file
only its user reads (a Kubernetes secret, say), and before or after `fctl
host install`:

```sh
fctl host claude set-token < /run/secrets/claude-token
```

The token stays on the machine; fctl never gets it. The workspace's admins
can take your runners out, which removes them, and they go when you leave
the workspace.

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
fctl projects set --setup-script 'npm ci' --instructions-file agents.md
fctl projects secrets set STRIPE_KEY       # asks for the value
fctl projects members add grace@example.com
```

Agents can use more repositories of the same account: under the project's
**Repositories** tab, choose any number of the repositories the workspace
may use, read only (a private dependency, say) or read and write, or
`my-org/*` to let agents read every repository the GitHub App can reach.
From the terminal: `fctl projects repos add my-org/my-lib --write`.

## 7. Start a session

Describe the task on the Agent Floor, the page the web app opens on, or on
the project page, or:

```sh
fctl run "Add a dark mode toggle to the settings page"
```

On a computer, the Agent Floor opens with the cursor in its task box. On a
Mac, ⌘← and ⌘→ still go back and forward while the box is empty; once it
has text, they move the cursor. From anywhere in the app, ⌘K and then Enter
take you to the box (Ctrl+K and Enter on Linux and Windows), and ⌘Enter
(Ctrl+Enter) starts the session. ⌘K opens the search that jumps to
sessions, projects and workspaces, and finds what sessions said (see
[Follow along and steer](#8-follow-along-and-steer)), and ⌘P and ⌘⇧P open
it too, as in code editors.

To start a session without leaving the page you are on, press N: the
Agent Floor's task box opens over the page, with what you wrote in it
before, in the project the page is about. While you type in a box, ⌘⇧O
(Ctrl+Shift+O) does the same, and Esc leaves the box for the other single
keys. G and then F goes to the Agent Floor, G and then N to the next
session in **Needs you**, and `?` (or ⌘/) lists every shortcut, those of
the page first. Shortcuts take the characters your keyboard's layout
types: on a Swedish or German keyboard, `?` is Shift and the key that
types it, and letters are wherever your layout has them.

The two lists by the Start button choose the model the session runs on
(Opus, Sonnet, Haiku or Fable, each the latest of its family) and its
effort level. They start on what you chose when you last started a session
in the project, else on the project's settings (its **Settings → Agents**
sets them for everyone). On a phone, Start is an arrow (↑), and the lists show
the model's name and the effort level's symbol, the one Claude Code shows
for it: ○ low, ◐ medium, ● high, ◉ extra high, ◈ max. In the terminal:
`fctl run --model haiku --effort low "…"`.

The same two lists sit beside the message box of a session you own, to
change them as it runs: plan at extra high effort, say, then build at
medium (`fctl edit my-app#12 --effort medium`). Claude works on the new
ones from its next step, and the session resumes on them. Claude's prompt
cache spares it reading the whole conversation again at each step: a new
effort level keeps it on the models Claude sends the level to with each
turn, Opus 5.5 among them, but a new model can't use it, so the page asks
before it switches (and before an effort change that would lose it too).

To show the agent what you mean, attach screenshots and files: with the
paperclip, by pasting them into the message box, or by dropping them on the
page (`fctl run -a screenshot.png "…"` in the terminal). The agent sees
them in its sandbox and gets screenshots in the message itself.

A task you write on the Agent Floor stays in your browser, with its project,
model, effort level and files, until a session starts with it: leave the
page, and it is there when you come back. So does one you write on a
project's page: each project keeps its own, apart from the Agent Floor's.

`fctl run` follows the session live in your terminal; Ctrl-C detaches and the
agent keeps working. The agent works on a branch named
`fctl/<number>-<title>`, commits as it goes, pushes, and opens a pull request
when it is done (projects can turn that off). In the project's other
repositories it changes, it uses the same branch and opens a pull request in
each. The session page and the session lists name every pull request the
session opens, with its repository (`fctl#120`, `design#14`), as does
`fctl show`. The session page's Changes section shows what the branch
changed since it left the base, the first ten files of each repository and
the rest a click away. A file opens the diffs of all of them, one after the
other, at that file (`c` on the session's page opens them at the first):
`j` and `k` go from file to file, `n` and `p` from change to change, and
⌘F (Ctrl+F) finds in all of them. They follow the
agent as it works: where you look, the lines it adds open and are written
in, and the lines you read stay where they are; a button at the top or the
bottom counts the changes it made out of sight and goes to them. Once its pull request is merged, Changes
says so, and fctl no longer pushes the branch. Once it understands the task it renames the session to what it
is working on and describes it in a sentence or two, so the Agent Floor shows
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

The Agent Floor lists the sessions at work under **Working now**, and the
latest that ended under **Recently**. When you are in more than one
workspace, choose one above them to list only its sessions: the task box
moves to that workspace too, to its first project in your sidebar where
you can start sessions. A project's page lists its sessions the same way,
with **Recently** for the ended ones that last moved in the past day and
**Older** for the rest, the most recent first. There
**Active** shows only the sessions at work, and **Mine** only the ones you
started.

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
  pinch or a double tap zooms in on it. The details beside the transcript
  show how full Claude's context is, with where Claude compacts the
  conversation to make room. On every page, the sidebar lists each
  project's sessions under it, in all your workspaces: those that haven't
  ended (starting, at work, waiting or paused), and for an hour those that
  stopped or failed, dimmer, the one you are on marked, so a click switches
  to another. A project on autopilot has its **Autopilot** above them,
  which opens the autopilot's own session: it says when it is at work,
  when it wakes next or that it is paused, and in blue how many things it
  asks you, which opens its Autopilot tab. On a computer, hover one that waits for a message, stopped
  or failed and click its eye to hide it from the sidebar in that browser,
  until its agent works again. Above them, **Needs you** gathers what waits on you in the
  sessions you started or follow, most pressing first: what an agent asks
  you, a failure, failed checks, a pull request an agent finished, a reply
  it waits for (unless its pull requests are all merged or closed and
  nobody wrote to it since but to answer what it asked: its work is
  done). Its count shows in the
  browser's tab, and on fctl's icon where you installed it. **Pull requests** opens the pull requests agents
  opened in the sessions you started, sorted by what each needs next; turn
  **Mine** off there for everyone's, and the sidebar counts them too. The
  browser keeps that choice, and the page keeps its tab and its filter
  while you look elsewhere. On a phone, tap the session's title for its
  details: its pull requests, the files changed, its branch and who else is
  watching. Type below to message the agent, with files attached if you
  like; it reads your message at its next step. **Interrupt** stops the
  current turn.
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

The session page opens with the session at a glance: its turns, the
agent's time, its steps, what it changed and its pull requests, and its
turns as a ribbon, each a click from its start. It reads a session in
turns: each message opens a card, what the agent did about it hangs from
the card on a thread, and each turn ends with how long it took, its steps
and the files it edited, each of which opens its diff; the ⋯ at a turn's
end copies it as Markdown, or a link that opens the session at that turn. The agent's plan
(its to-do list) stays beside the transcript, its steps checked off as the
agent goes. Above the message box, a line says what the agent does now and
for how long, led by the step of its plan it is at, with how far it got;
once it ends its turn and waits for you, the line says it's your turn.
While it waits for work it left running in the background (a dev server, a
build, a subagent), the line names it, and opens into a list of each task:
what it is, its command, for how long it has run, and a click to the call
that started it. That call's line says it runs in the background, and once
it ends, how long it ran.
Scrolled away from the newest work, a button goes back down to it and
counts what came meanwhile; back on the tab, a line marks where what came
while you were away starts. A bar under the header says which turn you are
in and lists them all; the details beside the transcript on a computer list
the newest, the one you are in and, a click away, the earlier ones; `j` and
`k` go from turn to turn. The speech bubbles in its
header (**Show only the conversation** in the ⋯ menu on a phone) leave out
the agent's work: what remains is what people asked and what the agent
answered.

To find what a session said, press ⌘F (Ctrl+F) on its page, or choose
**Find in this session** in its menu on a phone. The bar under its header
finds what you type in all the transcript says, folded or not: messages,
the agent's replies, the lines of its work and the commands behind them,
its thoughts and its subagents' work. It counts the matches from the
newest; Enter (or ↑) goes up to the one before, Shift+Enter (or ↓) back
down, and each opens what folds it. Going up past the oldest match loads
earlier activity to look in it too, and so does finding nothing in what
has loaded. A second ⌘F there opens your browser's own find. To find it in
any session you can see, type it in ⌘K: past the sessions whose titles
match, it lists those whose messages or agent's replies say it, and opens
one at that message with its find bar on it. From a terminal, `fctl
search "login redirect"`. Both find what you type as you type it, whatever
the case, the accents and the spaces between the words.

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

A session runs on the Claude subscription of whoever started it, and a
message keeps it there. When that subscription reaches its limit, the
session pauses until the limit resets. To carry on sooner on yours, or on
another connection of your own, choose **Continue on mine** above the
message box, or **Continue on my subscription** in the session's ⋯ menu
(`fctl take-over my-app#12`): the session starts again on the connection
you chose for its workspace, in the same conversation.

## 9. Work together

A workspace's admins invite people from its **People** tab (`fctl
workspaces invite grace@example.com`), by an address they sign in with;
they join it from their Agent Floor. An address the instance doesn't admit yet
needs an instance admin's invitation first (**Admin → People → Invite**, or
`fctl admin invite grace@example.com -w side-project`), which can invite it
to the admin's workspaces at the same time. One the allowlist admits but
nobody signed up with yet can be invited right away: its owner joins when
they sign up.

In a workspace, admins own every project, and members work in the projects
they are members of (a project's **Members**, or `fctl projects members`)
and in every project with the workspace's base role, which its admins
choose in its settings:

| Role | Can |
|---|---|
| Viewer | read sessions |
| Editor | start sessions and message agents |
| Maintainer | change settings and secrets, manage members |
| Owner | everything, including deleting the project |

Share a single session with someone else in the workspace from its
**Share** menu, or with `fctl share my-app#12 grace@example.com`. Everyone
watching a session sees who else is there.

A workspace has one primary admin, whom no other admin can demote or
remove; they hand the role to another admin under its **Settings**, as the
primary instance owner does under **Admin → People**.
