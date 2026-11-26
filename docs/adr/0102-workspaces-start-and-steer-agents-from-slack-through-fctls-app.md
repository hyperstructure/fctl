# 0102: Workspaces start and steer agents from Slack through fctl's app

2026-11-26 · accepted

## Context

Most organizations ask for work in Slack: a bug reported in #support, a
question in a thread, a request in a direct message. Starting an agent
from there meant leaving Slack for fctl, copying the context over, and
coming back to say what happened. A bot of one's own on a service account
and webhooks starts sessions from Slack ([A Slack bot of your
own](../chat-bot.md)), but its agents can't read the thread they were
asked from, and each organization builds and runs it. Claude's own Slack
connector would let agents read Slack, but it acts as the person, with
every private channel and direct message they can see: far more than an
agent should reach (#620).

Slack's only boundary is membership: an app reads every conversation it
is in, every message of a private channel it was invited to, and every
person's direct messages with it. What an agent reads there is fctl's to
narrow. Slack's API terms send commercially distributed apps through its
Marketplace: until an app is listed there, Slack lets it read threads and
channels (`conversations.replies`, `conversations.history`) once a minute
in each Slack workspace, 15 messages at a time, and keeps its Real-time
Search from it, while an organization's own, internal app keeps Slack's
full limits. A listing takes HTTP events (not Socket Mode), a review with
ten active installs, and, for agent apps, storing no Slack data.

The person who started the session that built this decided: the app
works on self-hosted instances and on app.fctl.sh alike, app.fctl.sh's set
up with Cloudflare secrets; app.fctl.sh knows people only by their link,
while a self-hosted instance's admins may let workspaces match people by
their verified address; a person links one Slack account in each Slack
workspace, and several in all; agents may read public channels from the
start, where a workspace's admins make them readable and a project lets
its agents; GitHub, Linear and Slack move into one Integrations tab of the
workspace page, as admins' tabs were full; Enterprise Grid org-wide
installs and Slack Connect channels wait.

## Decision

- **One app per instance**, an OAuth client with a bot user, `/fctl`, a
  message shortcut (**Send to fctl…**), a global one, a Home tab and
  messages in Slack's agent view, created from a manifest that Admin →
  Slack fills in with the instance's name and addresses. Its events come
  over HTTP, signed with its signing secret, or over Socket Mode with an
  app-level token. app.fctl.sh's is set up as any instance's, with the
  Worker's secrets: one code path, whose limits differ by what Slack
  counts the app as.
- **One installation per Slack workspace, shared**, as Linear's: Slack
  keeps one bot token for an app in a Slack workspace, so fctl workspaces
  connect a Slack workspace by installing the app from fctl, which takes
  an admin of the fctl workspace and someone Slack lets add apps, and
  share its installation, its token sealed with the master key. Each fctl
  workspace keeps its own choices there (`SlackSettings`), its own
  channels and its projects' choices. Slack revoking the app ends every
  connection to it; a workspace that disconnects hears nothing more from
  there; the last one to go uninstalls the app.
- **Sessions run as the person who asked, once fctl knows them.** People
  link their Slack account with Sign in with Slack, whose token fctl
  revokes at once, after confirming it's them where the instance asks for
  that; someone fctl doesn't know is asked, only them, to link it, and
  their request waits an hour. A person has one Slack account in each
  Slack workspace, and a Slack account is one person's. Knowing people by
  their address is opt-in twice, the instance's admins and then each
  workspace's, for organizations whose people sign in to Slack and fctl
  through the same provider; app.fctl.sh doesn't, as an address in Slack
  proves nothing a Slack workspace's admins can't change.
- **A thread is a session.** A mention in a channel, a thread of someone's
  direct messages with the app, `/fctl`, the shortcut and the Home tab
  start one, in the channel's project where admins mapped one, else the
  person's only one, else the one they choose; later messages in the
  thread are replies. Before it starts, only the person who asked moves
  it on; after, replies and answers reach the agent as their writers'
  messages only where those may message the session, and stay in the
  thread as information otherwise.
- **What the person asked is the task; the thread is information**, as a
  ticket's text is for Linear: the thread's earlier messages and a
  message sent with the shortcut come with the task marked as what the
  work is about, and the agent is told to do what the person asks.
- **Agents never hold a Slack token.** `fctl slack` goes through the
  control plane, as `fctl ticket` does, which looks each conversation up
  with Slack before it reads anything: a session reads its own thread,
  wherever it started, and the public channels its workspace's admins
  made readable (fctl's app joins them, which shows there), where its
  project reads them (on by default, since the admins chose them for
  agents); never more of a private channel or someone's direct messages
  than its own thread, never anyone else's direct messages with the app.
  It searches with Slack's Real-time Search where Slack lets the app,
  else by reading the readable channels' last week, and posts only in its
  own thread. Each session reads, searches and posts so many times an
  hour, so that one agent can't use up a Slack workspace's limits, which
  every session there shares.
- **One card, kept in place.** The thread gets a card at once, which
  fctl changes as the session moves: how it stands in words, the agent's
  plan as Slack's plan block draws it, its pull requests with their
  checks, and a link to the session. Only each turn's last message,
  questions with a button per answer, pauses and failures post new
  messages, so that the thread stays readable; the agent's step changes
  the card at most every 15 seconds, within Slack's limits. In direct
  messages, the agent's words stream as it writes them, and Slack's agent
  session shows how it stands, with a stop button.
- **What agents post is inert**: no link previews, so that a link an
  agent writes can't carry data out by unfurling, and Slack's special
  mentions taken apart, so that it notifies nobody.
- **Socket Mode in a Durable Object**, for instances Slack can't reach,
  on Cloudflare and the standalone server alike: one object holds the
  WebSocket, acknowledges each envelope at once, handles it as the HTTP
  endpoints do, and opens a new connection as Slack refreshes it.
- **Slack is also where people hear from fctl**: "Tell me in Slack" sends
  a person's notifications as direct messages from the app, with answer
  buttons, and a project posts the events it chooses to a channel, as
  webhooks name them, without a relay.
- **Each workspace chooses what transcripts keep of Slack**: what came
  from Slack stays, as anything else in a transcript does, unless the
  workspace asks fctl to drop what Slack's messages said once each
  session ends, keeping who wrote where.
- **Not yet**: Enterprise Grid org-wide installs, which fctl refuses
  (installs in each of an organization's Slack workspaces work), and
  channels shared with other organizations (Slack Connect), whose
  external people's messages and clicks would reach the app: fctl refuses
  mentions there, and never maps, reads or posts in them. Guests are
  refused, as Slack keeps them from agent apps.

## Alternatives

- **Claude's Slack connector.** It acts as the person, with every
  channel and direct message they see, for as long as they stay
  connected; nothing narrows it to the thread the work came from.
- **A bot of one's own, on service accounts and webhooks.** It stays the
  way for other chat apps, but every organization would build and run
  one, and its agents read only what it sends them.
- **Knowing people by their address everywhere.** No sign-in to ask for,
  but a Slack workspace's admins can set anyone's address, and a wrong
  match runs someone's request on another person's subscription.
- **Giving agents the bot token, or a person's token.** The bot is in
  every conversation the app was added to, every direct message with it
  included, and a person's token reaches all of theirs.
- **A new message for each update.** Simpler, but a long session would
  fill the thread, and Slack limits how fast an app posts in a channel.
- **HTTP events alone.** An instance on a private network would need a
  path open to the internet for Slack.
- **Storing nothing of Slack (zero-copy).** What a Marketplace listing
  asks of agent apps, but a session's transcript is how people follow and
  trust its work; dropping what Slack's messages said once sessions end
  is the step toward it that workspaces can take now.

## Consequences

- An instance has one Slack app. A self-hosted instance's is its
  organization's own, an internal app to Slack, with Slack's full limits
  and Real-time Search. app.fctl.sh's is distributed outside the
  Marketplace until Slack lists it: there, a task carries at most 15 of a
  thread's messages, `fctl slack` reads once a minute in each Slack
  workspace, and search reads the readable channels instead. Listing it
  is its own project: its events already come over HTTP, and it needs
  Slack's review and storing no Slack data.
- fctl workspaces connected to one Slack workspace share its
  installation: installing the app again replaces the token for all of
  them, and Slack revoking it, also as its installer leaves the Slack
  workspace or becomes a guest there, ends every connection.
- Each person links their Slack account once, on app.fctl.sh always.
  People a workspace matched by address stay known until they unlink.
- Agents never hold a Slack token, and read their own thread and the
  readable channels alone; a project can keep its agents to their thread.
- The card changes in place, and the thread gets one reply per turn.
- Socket Mode runs in a Durable Object, on Cloudflare and the standalone
  server alike, so a private instance needs no path open for Slack.
- Workspaces choose whether transcripts keep what came from Slack once
  sessions end; Claude's conversation in a session's snapshot keeps it
  until the snapshot goes.
- Enterprise Grid org-wide installs and Slack Connect channels aren't
  supported yet; guests are refused.
- The protocols grow by optional fields and values: `AgentCall::Slack`,
  which hosts and agents from before it refuse; `SessionSpec::slack`,
  which older hosts leave out; `Session::slack` and the transcript's
  Slack messages and notices; and `WebhookOrigin::Slack`, which older
  receivers read as `other`. Sessions from Slack still report back
  through older hosts, since the control plane posts in Slack.
- fctl stores bot tokens (sealed), people's Slack ids, names, addresses
  and avatars, the channels workspaces chose, which thread reaches which
  session, a waiting request's text for an hour, and what reaches
  transcripts. The privacy policy says so.
