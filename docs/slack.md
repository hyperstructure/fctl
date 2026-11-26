# Start agents from Slack

Mention fctl in a Slack thread, write to it in its messages, or send it a
message with **Send to fctl…**, and an agent starts on the work as you: in
a sandbox on your team's runners, on your Claude subscription. Its card in
the thread says how it goes and changes in place as it does, your replies
there steer it, its questions come with a button for each answer, and the
last message of each of its turns comes back where you asked. Agents read
the thread they started from, and public channels only where your
workspace's admins make them readable: never the rest of a private
channel, and never anyone else's messages with fctl.

fctl's Slack app works on app.fctl.sh and on instances people host
themselves alike ([Self-hosting](self-hosting.md#connect-slack-optional)).
This page is for the people who use it and the admins who connect it;
[Architecture](architecture.md#slack) says how it works, and
[ADR 0102](adr/0102-workspaces-start-and-steer-agents-from-slack-through-fctls-app.md)
why it works that way.

```text
#web-team
  Ada    @fctl the login redirect loops after a password reset: fix it, with a test   👀
    fctl  Fix the login redirect after a password reset
          ⚙ Running the tests
          web#12 · for Ada Lovelace
          2 of 3 done  ✓ Reproduce the loop  ▶ Fix the redirect  ○ Add a test
          ⤴ acme/web#45 Fix the login redirect · checks running
          [ Open in fctl ]
    fctl  🙋 The agent asked
          Should API clients keep the old redirect?
          Grace Hopper answered: Keep it for API clients
    fctl  I kept the old redirect for API clients and fixed it for the web, with a test…
```

The card is the first message of fctl's in the thread: the session's
title, how it stands, its number and whom it runs for, the agent's plan
(its todo list), its pull requests with their checks, and **Open in
fctl**. It changes in place as the session moves. Only the agent's
replies, its questions, and a pause or a failure post new messages, so
that the thread stays readable.

## Connect a Slack workspace

A workspace's admins connect it, on the workspace page's **Integrations**
tab under **Slack** (**Add to Slack**), or with `fctl workspaces slack
connect`. Slack asks to install fctl's app in one of your Slack
workspaces: whoever Slack lets add apps there installs it, and where your
Slack's admins approve apps, Slack asks one of them first. Back in fctl,
the workspace works in that Slack workspace, and its people link their
Slack accounts ([below](#link-your-slack-account)), you too.

- **Several fctl workspaces** can work in one Slack workspace, each
  connected by its own admins: they share fctl's app there, and each keeps
  its own settings and channels.
- **A self-hosted instance** needs a Slack app of its own first: its
  admins create it from the manifest under **Admin → Slack** (**Create the
  Slack app**), as [Self-hosting](self-hosting.md#connect-slack-optional)
  shows. app.fctl.sh has one.
- **fctl has to be in a channel** to hear it. Mapping a public channel to
  a project, or making it readable, has fctl join it. Elsewhere, Slack
  offers to invite fctl when someone mentions it in a channel it isn't
  in, and in a private channel, one of its members invites it (`/invite
  @fctl`).
- **Disconnecting** (**Integrations → Slack**, `fctl workspaces slack
  disconnect acme`) stops everything there from reaching the workspace's
  sessions, and forgets its channels. Once no workspace works in a Slack
  workspace, fctl uninstalls its app there.
- **Connecting again**, after Slack took fctl's access away (**Connect
  again**, `fctl workspaces slack connect --team acme`), installs the app
  in that Slack workspace again and keeps the workspace's settings and
  channels.

Org-wide installs in Enterprise Grid, Slack Connect channels and guests
aren't supported yet ([below](#not-supported-yet)).

## Link your Slack account

fctl starts sessions only for people it knows in a Slack workspace, as
them. The first time you ask fctl there, it sends you a message only you
see, with **Link your Slack account**: sign in with Slack, and your
request starts by itself, with 👀 on your message. It waits an hour for
you. To link it beforehand: **Settings → Slack**, or `fctl account slack
link`.

- Where the instance asks people to confirm it's them for sensitive
  actions, linking takes your passkey or a code, since what you ask in
  Slack then runs as you. fctl emails you once it's linked, where the
  instance sends email.
- fctl keeps who you are in that Slack workspace (your Slack id, name,
  address and picture), not a token: it revokes the sign-in's at once.
- You link one Slack account in each Slack workspace, in as many Slack
  workspaces as your workspaces work in. A Slack account is one person's:
  linking it from another fctl account moves it there.
- **Unlink** it in **Settings → Slack**, or with `fctl account slack
  unlink acme`: what you ask fctl's app there starts nothing until you
  link it again.

**Matching by address.** An address in Slack proves nothing to fctl: a
Slack workspace's admins can change people's addresses there. So
app.fctl.sh knows people only by their link. A self-hosted instance whose
people sign in to Slack and to fctl through the same provider can let
workspaces know people by their address instead: its admins allow it
under **Admin → Slack** (`fctl admin slack --match-email on`), and each
workspace's admins turn it on for a Slack workspace (**Know people by
their verified address**, `fctl workspaces slack set --match-email on`).
Someone who hasn't linked an account is then the fctl account whose
verified address is their Slack address, where that account is a member
of a workspace that turned it on, unless they are a guest there. Their
**Settings → Slack** shows the match. It lasts only while it is allowed:
once the instance or every such workspace turns matching off, fctl forgets
it the next time they ask, and asks them to link their account.

## Start a session

| Where | How | Its card and replies go |
|---|---|---|
| A channel fctl is in, public or private | Mention fctl with what to do, in a new message or in a thread: `@fctl the checkout page 500s since this morning: find out why` | To that thread |
| fctl's messages (the app, in Slack's sidebar) | Write to it what to do | To that thread, a session of its own that only you see, with the agent's words as it writes them |
| A channel | `/fctl <what to do>`; where you could start sessions in several projects, a form asks which | To a new thread there, which fctl starts with what you asked; to your messages with fctl where it isn't in the channel |
| Any message in a conversation fctl can see | **Send to fctl…** in the message's menu: a form with the message, a project and what to do | To the message's thread, where fctl is in the channel and **Post its card and replies in this thread** stays ticked; else to your messages with fctl |
| fctl's **Home** tab, or the **Start an fctl session** shortcut | **Start a session**: a project and what to do | To your messages with fctl |

`/fctl status` lists, only for you, what needs you and what runs, and the
Home tab shows the same with your recent pull requests. Slack runs no
app's slash commands in threads: mention fctl there instead.

**Which project.**

1. In a channel its workspace's admins mapped to a project, that project:
   "Mentions in #web-team start sessions in web".
2. Else the only project you can start sessions in, among the projects of
   the fctl workspaces that work in this Slack workspace and let such
   requests start sessions.
3. Else fctl asks which, in a message only you see. A workspace's admin
   can tick **Remember for #channel** there, which maps the channel.

In your messages with fctl, it's the project you keep for them (**Settings
→ Slack**, `fctl account slack dm-project web`), else the only one, else
your choice. You start sessions where you're an editor of the project, or
more, as in fctl.

**What the agent gets.** What you wrote is its task: your instructions.
The thread's earlier messages come with it (up to 50 of them, and 30,000
characters), and with **Send to fctl…** the message you sent and its
thread, as information from the people who wrote them: the
agent takes them as what the work is about, not as instructions, even
where they ask for something. Mentions read as names, and links as their
words and address. Files attached in Slack aren't passed on, and the
agent sees a reply's files only by their names: send files in fctl
instead (the session's page, `fctl send -a`), where they reach it.

The session is yours, as one you start in fctl: it runs on your Claude
subscription, in the project's sandbox with its GitHub access, on a branch
of its own, titled after the first line of what you asked, and you follow
it. Webhooks say it `started_by` `slack`.

## Follow up and answer

- **Reply in the thread.** Your reply reaches the agent as your message,
  at its next step, where you may message the session in fctl: you're an
  editor of its project, or more, or it's shared with you as an editor.
  fctl reacts to it with 👀. A stopped session resumes for it; a failed
  one takes no messages, and the thread says so: resume it in fctl.
- **Others in the thread.** Replies from people who may not message the
  session stay in the thread, where the agent can read them, as
  information. Someone who mentions fctl but may not message the session
  hears why, alone. Someone fctl doesn't know who mentions fctl gets the
  link to link their account, and their message reaches the agent once
  they do, within the hour.
- **Before it starts.** While fctl waits for you to link your account or
  choose a project, only you move the request on: a reply of yours adds to
  it and tries again. Anyone else's would start it on your subscription.
- **Answer its questions.** When the agent asks with `fctl ask`, its
  question comes with a button for each answer. A click answers as you,
  where you may message the session, and the question then shows who
  answered what; anyone else hears why, alone. A reply in the thread
  answers it too.
- **Stop it.** While the agent works, Slack shows it working in the
  thread, with a stop button that interrupts its turn, as a stop in fctl
  does. To end the session, stop it in fctl.

The card's status, in words:

| The card says | It means |
|---|---|
| Waiting for a runner | It waits for a runner with room |
| Starting its sandbox | Its sandbox starts |
| What the agent does ("Running the tests") | The agent works: its plan and pull requests show below, with their checks |
| Waiting for your answer | The agent asked something |
| Waiting for a reply | Its turn ended |
| Done | Its pull requests merged |
| Paused until … | Its Claude subscription reached a usage limit: it carries on by itself then, a time Slack shows each reader in their own time zone |
| Failed: … | Why, in a line; people who manage it resume it in fctl |
| Stopped: reply here to carry on | Someone stopped it, or it stopped idle |

## What agents read, and never read

An agent never holds a Slack token. `fctl slack` in its sandbox asks fctl,
which checks each conversation with Slack before it reads anything:

| Agents read | Agents never read |
|---|---|
| The thread their session started from, in a channel, a private channel or your messages with fctl | Any other message of a private channel |
| The public channels your workspace's admins made readable, where their project reads them: a message and its thread, a channel's latest messages, and a search of them where the workspace allows it | Anyone's messages with fctl, and group direct messages, other than their own session's thread |
| | Channels shared with other organizations (Slack Connect) |

They post only in their own thread: fctl posts the last message of each
of their turns, and they post a quick word themselves (`fctl slack
reply`), such as a link people asked for, where the workspace lets them.
Whatever they write notifies nobody: `@channel`, `@here`, `@everyone`,
user groups and people's mentions reach no one, and links don't unfurl
into previews. In an hour, a session's agent reads Slack at most 120
times, searches it at most 30 times and posts at most 20 times. Review
agents reach nothing in Slack.

The agent is told the same: people's replies reach it as their messages
where they may message the session, and everything else from Slack is
information from the people who wrote it. Outside your messages with
fctl, it's told that everyone in the channel reads the thread, so that it
posts no secrets there.

## Settings

### What a workspace allows

A workspace's admins choose, for each Slack workspace it works in, on the
**Integrations** tab under **Slack** or with `fctl workspaces slack set`:

| Setting | Default | What it does |
|---|---|---|
| Mentions in channels (`--channels`) | on | Mentions in channels, `/fctl` and **Send to fctl…** start sessions |
| Direct messages (`--direct-messages`) | on | Writing to fctl in its messages starts sessions, and so do its Home tab and the **Start an fctl session** shortcut |
| Search the readable channels (`--search`) | on | Agents search the readable channels, besides reading them |
| Agents' own replies (`--agent-replies`) | on | Agents post in their own thread with `fctl slack reply`; off, only fctl posts there |
| Notifications in Slack (`--notifications`) | on | People who choose **Tell me in Slack** get their notifications from fctl's app |
| Projects' events (`--project-events`) | on | Projects post the events they choose to a channel |
| Know people by their verified address (`--match-email`) | off | People who haven't linked an account are known by their address, where the instance allows it |
| Transcripts (`--transcripts keep` or `drop`) | keep | **Drop them once a session ends**: fctl drops what Slack's messages said from sessions' transcripts ([below](#transcripts)) |

### Channels

- **Map a channel to a project**: "Mentions in #web-team start sessions in
  web" (`fctl workspaces slack map web-team web`; `none` for no project of
  its own). fctl joins a public channel it maps. The list shows public
  channels alone: for a private one, invite fctl there, mention it, and
  tick **Remember for #channel** as you choose the project.
- **Make public channels readable**: the agents of the workspace's
  projects that read Slack read them (`fctl workspaces slack readable
  incidents deploys`, and `--off` to stop). fctl joins each, which shows
  there ("fctl joined #incidents"). A private channel can't be readable:
  there, agents read their own thread alone.
- The page says what agents can see in a sentence: "Agents read the thread
  they started from, and #incidents and #deploys, which they can search."

The shell takes `#` at the start of a word for a comment, so give the CLI
channels' names without it, or quote them (`'#web-team'`).

### What a project does

A project's owners and maintainers choose, in its settings under
**Slack** or with `fctl projects slack`:

- **Agents read the readable channels**: on by default, since the workspace's
  admins chose those channels for agents to read. Off, its agents read
  their own thread alone (`--read-channels off`).
- **Post events to a channel**: a channel of a Slack workspace the
  project's workspace works in, and the events, among those webhooks get:
  pull requests ready for review (`session.ready_for_review`) and failures
  (`session.failed`) unless you choose others. Each comes as a line, with
  the session's title and **Open in fctl**. fctl joins a public channel,
  found by its name; a private one's members invite fctl first, and you
  give its ID. Only where the workspace's admins let projects post events
  in Slack.

```sh
fctl projects slack -p web --events web-alerts
fctl projects slack -p web --events web-alerts --kinds session.failed,session.question
fctl projects slack -p web --events none
```

### Yours

**Settings → Slack** lists your Slack accounts, links and unlinks them,
and keeps the project your messages with fctl start sessions in (`fctl
account slack dm-project web`, or `--ask` to choose each time).

## Notifications in Slack

Turn on **Tell me in Slack** in **Settings → Notifications** (`fctl
notifications set --slack on`), and fctl's app sends your notifications in
your messages with it too: a session you follow finished its turn, asks
you something, paused on a usage limit or failed. A question comes with a
button for each answer, and a reply in a notification's thread reaches the
session as your message. Each goes to the Slack workspace the session's
workspace works in, where you linked your account and its admins let
notifications go there. Several that come close together arrive as one
list, at most one a minute, as in your browsers.

## Transcripts

A session keeps what came from Slack in its transcript, as anything else
there: the messages fctl passed on, the thread's earlier messages in its
task, and what its agent read. A workspace that would rather not chooses
**Drop them once a session ends** under **Transcripts** (`fctl workspaces
slack set --transcripts drop`):
within about an hour after a session ends (it stopped, failed or was
archived), fctl replaces what each message from Slack said with who wrote
it where, and what `fctl slack` read with a note that it was dropped, in
the transcript and in search. A session that carries on later is dropped
again once it ends. Claude's own conversation keeps them as long as the
session keeps it: in its sandbox while its runner keeps that, and in its
encrypted snapshot until the snapshot expires or the session is deleted
([Session snapshots](architecture.md#session-snapshots)). What the agent
wrote, its replies in Slack included, stays.

## Limits

- **On app.fctl.sh**, Slack counts fctl's app as one distributed outside
  the Slack Marketplace until it's listed there: it lets fctl read threads
  and channels once a minute in each Slack workspace, 15 messages at a
  time, and keeps its Real-time Search from it. A session's task then
  carries at most 15 of the thread's messages, `fctl slack` says when to
  try again, and its search reads the readable channels' last week
  instead.
- **On a self-hosted instance**, the app is your organization's own, an
  internal app to Slack: Slack's full limits, and Real-time Search where
  the message that started the session lets fctl search.
- A reply is cut at 3,000 characters, with a link to the rest in fctl. In
  your messages with fctl, the agent's words stream for at most nine
  minutes and 11,000 characters a turn; past that, its last message comes
  as a reply.
- A request waits an hour for you to link your account or choose its
  project; after that, ask again.
- The card shows the agent's latest step at most every 15 seconds; other
  changes show at once.

## Not supported yet

- **Enterprise Grid org-wide installs.** fctl refuses an install for a
  whole Enterprise Grid organization, and events from one: install fctl's
  app in each of its Slack workspaces, and connect each.
- **Slack Connect channels**, shared with other organizations. A mention
  there gets an answer that only its author sees, saying so; fctl maps,
  reads and posts in none of them.
- **Guests** of a Slack workspace: Slack keeps them from agent apps, and
  fctl refuses their requests.
- **Files** attached in Slack: the agent sees a reply's files by their
  names alone.
- **Slack's Tasks tab** stays off.

## Troubleshooting

| You see | Why | What to do |
|---|---|---|
| Nothing when you mention fctl | fctl isn't in the channel, or the instance takes no events from Slack | Invite fctl, as Slack offers; an admin checks **Integrations → Slack**, which says when Slack's events don't reach the instance |
| "fctl doesn't know who you are yet" | You haven't linked your Slack account | **Link your Slack account**: your request starts once you do, within the hour |
| "there's no fctl project here you can start sessions in" | No project you're an editor of, in a workspace that works in this Slack workspace and lets such requests start sessions (**Mentions in channels**, or **Direct messages**) | A project's owners make you an editor, or the workspace's admins turn that on |
| "mentions here start sessions in Web, which you can't start sessions in" | The channel maps to a project you aren't an editor of | Its owners make you an editor of it |
| "connect your Claude subscription in fctl first" | Sessions run on yours | **Settings → Claude**, then ask again |
| "fctl couldn't start on this: …" | What it says | Reply in the thread to try again, within the hour |
| "Only the person who asked in this thread can start its session" | Someone else's request waits there | Mention fctl in a new thread |
| "you can't message this session in fctl" | You may not steer it | Its owners share it with you |
| "The fctl session failed, so your reply didn't reach its agent" | It failed | Resume it in fctl |
| "Slack's guests can't use fctl's agents here" | You're a guest of the Slack workspace | A member asks instead |
| "fctl doesn't work in channels shared with other organizations (Slack Connect) yet" | A Slack Connect channel | Ask in one of your own channels, or in your messages with fctl |
| "fctl can't see this conversation" | A private channel fctl isn't in, or someone else's direct messages: fctl can't tell whether it's shared with another organization, so it takes nothing from it | Invite fctl to the channel (`/invite @fctl`), or write to fctl in your messages with it |
| "fctl couldn't post the agent's reply in the thread…", in your messages with fctl | fctl was removed from the channel, or the channel was archived, after the session started there | Invite fctl again to carry on in the thread; meanwhile the session's replies and questions come to you |
| **Integrations → Slack** says its access was revoked | Slack uninstalled fctl's app or revoked its token: someone removed it, or the person who installed it left the Slack workspace or became a guest there | An admin connects it again, which installs the app again |
| Connecting comes back with "That sign-in took longer than ten minutes", or "fctl doesn't take Enterprise Grid installs for a whole organization yet" | The sign-in waited too long, or Slack installed the app for a whole Enterprise Grid organization | **Add to Slack** again, and choose one Slack workspace |
| `fctl slack` says Slack limits how often fctl reads | app.fctl.sh's limits, [above](#limits) | The agent tries again a minute later |
