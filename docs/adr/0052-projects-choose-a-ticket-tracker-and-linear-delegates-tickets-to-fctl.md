# 0052: Projects choose a ticket tracker, and Linear delegates tickets to fctl

2026-10-23 · accepted · fctl moves tickets to In Review and Done as their sessions' pull requests come up and merge, and to In Progress once a mention's agent changes something, since [0063](0063-fctl-moves-linear-tickets-through-their-teams-workflow-as-sessions-work.md)

## Context

Agents read and wrote GitHub issues wherever their tokens could, which
were the repositories they push to, and nothing told them which tracker a
team uses. Teams that track their work in Linear had no way in: their
tickets lived where agents couldn't reach them, and they started sessions
by hand, pasting tickets.

The person who started the session that built this asked for a tracker
per project, none, GitHub Issues or Linear, which agents are told to use
and fctl supports; for none to keep agents away from every tracker,
GitHub's issues included; for GitHub Issues to cover every repository
agents write to or one of them; for implementations that fit trackers to
come (Jira, Shortcut, ClickUp), with parents, sub-tickets and fields of
their own; and for teams that use Linear to delegate tickets to fctl
there, with a Linear app installed for the project, each run as the right
person, without anyone putting a ticket on someone else's session. They
had made a Linear OAuth app.

## Decision

- **A tracker per project, in its settings** (`ProjectSettings::tickets`):
  none, GitHub Issues (of the repositories agents write to, or of one of
  them) or a Linear team, optionally narrowed to one of its projects.
  Projects without one keep GitHub Issues of their writable repositories,
  as agents always worked, so nothing changes until someone chooses. A
  tracker a newer version stores that this one doesn't know reads as
  none.
- **Tokens follow it.** GitHub tokens carry the issues permission only
  where the tracker says: as before for GitHub Issues of the writable
  repositories; on a write token for one repository alone when it is the
  one chosen; nowhere for none or Linear. A token has one set of
  permissions for all its repositories, so the issues of one among
  several go on no session token. Public repositories' issues are public
  anyway: there the agents' instructions are what keeps them away, and the
  docs say so rather than claim more.
- **One way in for every tracker: `fctl ticket`, through the control
  plane.** The control plane holds the tracker's credentials and keeps each
  call to the project's choice, refusing tickets elsewhere as ones that
  don't exist. Its requests reach the control plane as an agent call whose
  body hosts and fctl-agent pass on unread, so that what `fctl ticket`
  does grows with the control plane and the sandbox's fctl alone. A
  `Ticket` holds what trackers share, with a state of a kind (triage,
  backlog, unstarted, started, completed, canceled) and a parent and
  children, and the rest as named fields: a tracker added later maps onto
  it. Agents that reach GitHub Issues with `gh` keep doing so; `fctl
  ticket` covers the issues of one repository among several, and Linear.
  Linear's token never enters a sandbox: it reaches the whole Linear
  workspace, all its teams, as fctl's app.
- **What agents are told comes from the control plane** (`SessionSpec::tickets`),
  which knows the trackers, so that new trackers and new wording need no
  new agents.
- **Linear through its agent platform.** fctl's app is installed in a
  Linear workspace with `actor=app`, which gives it a user there that
  people assign issues to (it becomes their delegate) and mention; both
  start an agent session in Linear, which fctl answers with Linear's own
  activities: a thought as it starts, steps while the agent works, its
  pull requests, and each turn's last message as the response, or a
  question. Replies in that thread reach the session's agent, and Linear's
  stop interrupts it. This is how Linear wants agents to work, and it
  carries who asked, which labels, status changes and comments alone don't.
  Delegating and mentioning are the triggers; a label (`explore` unless a
  project names another) asks for a plan before any change.
- **One installation per Linear workspace, shared.** Linear keeps one
  token for an app's user, so installing the app again replaces it. fctl
  workspaces connect a Linear workspace by installing the app from fctl,
  which takes an admin of both, and share its installation; projects
  choose a team among those the app sees. A workspace that disconnects one
  reaches nothing there any more, whatever others do.
- **Sessions run as the person who asked, once fctl knows them.** People
  link their Linear account with a Linear sign-in of their own, which fctl
  keeps the identity of and revokes at once. A delegation from someone fctl
  doesn't know waits for them to link it, with Linear's own button in the
  agent session, and starts as soon as they do. The session runs on their
  Claude subscription, in the project that takes the issue's team, only
  while they may start sessions there, so nobody puts a ticket on anyone
  else's session; replies reach the agent as their writers' messages,
  under the same rules as messages in fctl.
- **Status moves where Linear says.** A delegated ticket moves to its
  team's first started state as its session starts, as Linear asks of
  agents (projects can turn that off). Branches carry the ticket's key, so
  Linear's GitHub integration links pull requests and moves tickets on as
  they open and merge, as each team configures; agents don't move tickets
  themselves unless asked.
- **Not too much.** One response per turn, cut at 3,000 characters; steps
  at most every two minutes, ephemeral, saying what the agent does without
  its commands or addresses; in an hour, an agent comments at most 30
  times, files at most 20 tickets, changes at most 60 and reads at most 300
  times, so that one workspace's agents can't use up the API of a Linear
  installation others share.

## Alternatives

- **Matching Linear users to fctl accounts by email.** No sign-in to ask
  for, but an address in Linear proves nothing to fctl, and a wrong match
  runs someone's ticket on another person's subscription.
- **A Linear installation per project.** Simpler to explain, but a second
  project would install the app again and break the first's token.
- **Labels or states that start sessions.** Possible with Linear's issue
  webhooks and `agentSessionCreateOnIssue`, but they don't say on whose
  subscription, and they start work nobody asked an agent for. Delegating
  says both; label triggers can come later on top of it.
- **Giving agents a Linear token, or Linear's MCP server.** It would reach
  every team of the Linear workspace, outlive the session, and act as a
  person rather than as fctl's agent.
- **A per-repository issues token for `gh`.** gh holds one token per host,
  so it would need a token of its own beside the read-only one, more
  protocol for the hosts, and still a second way in; the control plane
  covers the case with `fctl ticket`.

## Consequences

- New projects and existing ones behave as before until someone chooses a
  tracker. Choosing one reaches running sessions' tokens within the hour.
- Hosts and agents from before this pass none of it to Claude, and can't
  run `fctl ticket`; their tokens still follow the tracker, and sessions
  that tickets start still report back, since the control plane does that.
- Each person who delegates links their Linear account once. Until they
  do, what they delegate waits for an hour.
- fctl stores Linear's tokens (sealed), people's Linear ids, names,
  addresses and avatars, and which Linear agent session reaches which
  session; tickets' content reaches sessions' transcripts. The privacy
  policy says so.
- A tracker added later needs its own client and mapping onto `Ticket`,
  and its own triggers; the setting, `fctl ticket`, the transcript's
  ticket messages and notices, and the tokens' rules stay as they are.
