# Security

fctl runs code-writing agents with access to your repositories and your
Claude subscription. This page says what it protects, how, and what it
trusts.

## What it protects

| Asset | Where it lives | Who can use it |
|---|---|---|
| Passwords | argon2id hashes in D1 | nobody can read them back |
| Authenticator app secrets (two-factor) | encrypted (AES-256-GCM) in D1 | the control plane, to check codes |
| Recovery codes | SHA-256 hashes in D1 | nobody can read them back |
| Passkeys | public keys in D1; the private key never leaves the person's device or password manager | the person, with their fingerprint, face or screen lock |
| Sessions and tokens | SHA-256 hashes in D1; the secret only with its holder | the holder |
| Claude credentials | encrypted (AES-256-GCM) in D1, or, for tokens people keep on their runners, only in a file on those runners (mode 0600) | sessions their owner starts |
| Project secrets | encrypted in D1 | that project's sessions, as environment variables |
| Profile pictures | images in D1 | anyone signed in to the instance, at an address with the person's user ID |
| Workspace avatars | images in D1 | the workspace's members, people it invited, instance admins |
| Attached files | the project's Durable Object | people who can see the session they were sent in; its runner and agent |
| Published screenshots | the project's Durable Object, found by the SHA-256 of their links' tokens in D1 | anyone with a link, until it expires |
| GitHub App key | a Worker secret (outside Cloudflare, the server's environment) | the control plane, to mint short-lived tokens |
| GitHub webhook secret | a Worker secret (outside Cloudflare, the server's environment) | the control plane, to check that deliveries come from GitHub |
| Linear app's client secret and webhook secret | Worker secrets (outside Cloudflare, the server's environment) | the control plane, to install fctl's app in Linear, renew its tokens and check that deliveries come from Linear |
| Linear tokens (fctl's app in each Linear workspace) | encrypted (AES-256-GCM) in D1 | the control plane, for the projects of the fctl workspaces connected to that Linear workspace; never sandboxes |
| Web Push (VAPID) key | encrypted in D1 | the control plane, to sign notifications |
| Master key | a Worker secret (outside Cloudflare, the server's environment, or a file it made in its data directory) | the control plane |

Encrypted values are bound to their row (the AES-GCM associated data names the
owner and purpose), so a value copied between rows does not decrypt.

## Access

- Accounts exist only for allowlisted emails or domains. Each of an
  account's addresses was invited (or admitted) on its own, and any of them
  signs in to the same account, with the same factors. Removing an
  allowlist entry disconnects the addresses it admitted from their
  accounts, which leave the workspaces they are in as them, and disables
  the accounts none of whose addresses is admitted any more, revoking their
  tokens. Revoking an invitation takes back the entry it added, and
  demoting or removing an admin turns the entries that admitted their
  addresses as an admin into a member's, so that a Google sign-up, which
  needs no invitation, can't bring them back as one.
- **Workspaces** keep their projects, sessions, files and secrets to their
  members: people outside one are told its projects and sessions don't
  exist, whatever they started there or was shared with them, and instance
  admins see inside only the ones they are members of. Starting a session
  makes someone its owner only while they may start sessions in its
  project: one made a viewer of it, or taken out of it, can't go on
  messaging its agent, resuming it or sharing it. On an instance that
  sends email, the password reset and invitation links admins make go to
  the addresses alone, never to the admin: a reset link signs in as its
  person, and an invitation's joins the workspaces that invited its
  address. Without email, admins hand those links over, so whoever runs
  such an instance can sign in as its people. A workspace sees
  only the address each member joined as. Only its admins create its
  projects, so its members' agents reach the repositories of the projects
  the admins made and opened to them, not every one its GitHub accounts
  cover.
- Browser sessions are `__Host-` cookies (`Secure`, `HttpOnly`,
  `SameSite=Lax`). An instance served over plain http at an address other
  than localhost, which only [a control plane of your own](#the-control-plane-without-cloudflare)
  can be, sets them without the prefix and `Secure`, which browsers refuse
  there. Requests that change state must come from the app's own
  origin (`Origin` / `Sec-Fetch-Site`) and carry JSON; WebSocket upgrades are
  origin-checked too.
- **Two-factor sign-in.** With an authenticator app set up (Settings, or
  `fctl account two-factor enable`), a sign-in with the password, and with
  Google when you or the admins choose that, takes a code from the app
  (TOTP) or one of ten recovery codes. Each code works once, and five wrong
  ones in a row make codes from the app wait a minute, doubling with each
  further one up to an hour; recovery codes still work meanwhile, and you
  get an email, since whoever it was knew your password. A reset link's new
  password takes effect only once a code checks out, so the link alone
  signs nobody in. Replacing the app, turning it off and getting new
  recovery codes take a current code; setting up an app takes the password
  unless the browser signed in within 15 minutes (with a CLI or access
  token, always), and on an account without a password, that sign-in.
  Admins can require it:
  accounts that sign in with a password, and with Google sign-ins asking
  too every account, then can do nothing until they set it up, in the
  browser or with their CLI and access tokens. An attacker who has the
  password of an account that hasn't set it up yet could set it up first;
  the email about it (when email is set up) and the audit log show that,
  and admins reset two-factor for people who lost their phone (the
  primary instance owner for admins; the owner's own takes a recovery
  code). A workspace's admins can require it of its members in the same
  way: a member without a second factor can't open the workspace until
  they set one up, and their sign-ins ask for it.
- **Passkeys** sign in without a password or code: the device checks it's
  the person (fingerprint, face, screen lock or PIN), and the signature
  works only for this instance's host name, so a fake site can't use it.
  fctl keeps the public key, each challenge works once for ten minutes, and
  a counter that goes back (a copied passkey's would) refuses the sign-in.
  Adding one takes the password unless the browser signed in within 15
  minutes, so a stolen session can't add a way in, and emails the person.
  Where two-factor is required, a passkey also counts as the second factor.
- **Sensitive actions take a passkey or a code**, where the primary
  instance owner turns that on (off by default): inviting and promoting
  admins, handing over the primary owner's and workspaces' primary admin
  roles, turning two-factor sign-in or this off, resetting someone's
  two-factor sign-in, removing a passkey, adding or registering a runner
  for a workspace and replacing its token, new access tokens and CLI
  sign-ins, exporting your data, deleting accounts, projects and
  workspaces, and, without email, making someone a password reset link.
  So a stolen browser session, CLI or
  access token can't do them alone. A passkey (which must verify the
  person), a code from the authenticator app or a recovery code confirms
  it, for 15 minutes, for the browser, CLI or token that confirmed; a
  sign-in with a passkey or a second factor counts. People with neither a
  passkey nor an authenticator app aren't asked, and only the primary
  instance owner turns it on or off, which takes confirming it too.
- The CLI signs in with a device flow (you approve a code in the browser) and
  gets a revocable token; Settings lists every device and token.
- **Access tokens** for scripts, CI and bots can be limited to reading,
  running sessions or everything their person may, to some workspaces and
  projects, and in what their sessions' agents may do on GitHub. A limited
  token reaches projects and sessions alone: one rule in the router refuses
  it the account, workspaces' people and settings, Claude connections,
  other tokens and administration before any handler runs, so a token that
  leaks from CI can't make itself another, and its roles in its projects
  are capped. It can't message or resume a session whose agent started
  able to do more on GitHub than it allows, and only a token that runs
  sessions may hold its agents lower: one that does everything could
  change the settings that steer them. Make them expire, and revoke one that may have
  leaked in Settings or with `fctl tokens revoke`.
- Sign-in, signup and reset endpoints are rate limited per address and per
  account; unknown emails take as long to fail as known ones. Signing up
  without an invitation, asking for a password reset or for early access,
  and inviting an address to a workspace answer the same whether the
  address has an account or can sign in, so that none of them tells who
  uses the instance (instance admins, who see that anyway, excepted).
- The web app ships a strict Content-Security-Policy (`script-src 'self'`, no
  inline scripts), `frame-ancestors 'none'`, `Cross-Origin-Opener-Policy:
  same-origin`, HSTS, and renders agent output as text (Markdown is parsed
  into elements, never injected as HTML). Links in what agents and other
  people write open web pages in a new tab, and never the app's sign-in
  pages or API, however they name them (`textLink`).
- Notifications go through each browser's push service (Google's for
  Chrome, Mozilla's for Firefox, Apple's for Safari, Microsoft's for Edge),
  encrypted for that browser alone (RFC 8291): the service sees when one
  goes and how big it is, not what it says. They name the session and
  quote the start of the agent's last message, and show like any
  notification, on a lock screen too; people choose what notifies them in
  Settings. The control plane only sends to `https://` addresses of named
  hosts on port 443, follows no redirects from them, and sends to at most
  20 browsers per person.
- File uploads are `application/octet-stream` bodies, which browsers cannot
  send cross-site without asking first, and pass the same origin check.
  Attached files are served with `nosniff` and a sandboxing
  Content-Security-Policy; only PNG, JPEG, GIF and WebP display in place,
  everything else downloads, and only those and the videos the app plays
  keep their type, the rest being `application/octet-stream`, so an
  uploaded HTML, SVG or JavaScript file never runs as part of the app.
- GitHub's webhook (`POST /api/v1/github/webhook`) takes only deliveries
  signed with the webhook's secret (HMAC-SHA256, checked in constant time),
  and none without the secret. It takes from them only which pull requests
  or commits changed, then asks GitHub itself what became of the pull
  requests sessions list, so a replayed delivery changes nothing.
- Linear's webhook (`POST /api/v1/linear/webhook`) takes only deliveries
  signed with the webhook's secret (HMAC-SHA256, checked in constant time)
  whose `webhookTimestamp` is within a minute of the control plane's
  clock, up to 5 MB, and none without the secret. A delegation or mention
  there starts a session only as the fctl account that linked the Linear
  account it came from, in a project that takes the issue's team, of an
  fctl workspace connected to that Linear workspace, and only while that
  account may start sessions in the project, so nobody in Linear puts a
  ticket on someone else's session or subscription. A delivery for a
  Linear agent session fctl has seen starts nothing new; before its
  session starts, only the person who made it can start it again or
  choose its project, and once it has, replies reach its fctl session
  only from linked accounts that may message it. People link a Linear
  account with a Linear sign-in of their own, whose token the control
  plane uses once to read who they are and then revokes, so an address in
  Linear proves nothing on its own; a sign-in as fctl's app links nothing.
- Screenshots agents publish for pull requests are public to anyone who
  has a link, which is how GitHub's image proxy fetches them. A link holds
  256 random bits, of which D1 keeps only the SHA-256. The control plane
  publishes only PNG, JPEG, GIF and WebP images, as their bytes say, up to
  5 MB, and serves them with `nosniff`, a sandboxing
  Content-Security-Policy and `noindex`. Links stop working when they
  expire (30 days unless the project chooses 24 hours to 60 days), when
  people who manage the session delete them, and while the instance's
  admins or the project's maintainers turn publishing off; it is off for
  both until they turn it on. Anyone a link is shared with, or who can read
  the pull request, can pass it on.

## Sandboxes

Each session runs in its own Linux VM (Apple container or Lima) or container
(Docker). The agent runs Claude Code with permission prompts off, because the
sandbox is the boundary: it can do anything inside its VM, and nothing
outside it except through these channels:

- **Lima VMs** get none of the mounts of Lima's templates, which give
  each VM the home directory of whoever runs the host (read only) and a
  `/tmp/lima` every VM writes to, and none of their port forwards, which
  put a VM's services on the host's loopback, where other VMs reach them.
  A VM still reaches the host's own loopback services
  (`host.lima.internal`), as Docker Desktop's containers do theirs.
  Lima hosts of fctl 0.14.1 and earlier cloned VMs that could read that
  home directory, with the host's runner token and the CLI's credentials.
  On such a host, remove its runner and install the host again (`fctl
  host uninstall`, `fctl host install`), sign the CLI in again (`fctl
  login`) and revoke its old tokens in Settings, and replace any other
  keys kept in that home directory.

- **Network.** Sandboxes have outbound network access (to install
  dependencies and reach GitHub and Claude). Do not run hosts on networks
  where an outbound connection from an untrusted process is a problem.
  Docker sandboxes join a network of their own, `fctl-sandboxes`, on which
  they reach the internet but not each other, so a session can't reach
  what another session on the host serves, which may be another
  workspace's; a Docker that can't make the network leaves them on its
  default one, and the host says so in its log. With Docker Desktop and
  OrbStack, containers still reach the host's loopback
  (`host.docker.internal`). Apple's `container` VMs share its network: on
  a runner that serves more than one workspace, keep to Docker where that
  matters.
- **GitHub.** Installation tokens for the project's repositories and no
  others, within the GitHub accounts and repositories its workspace may use,
  valid for an hour and refreshed on demand
  ([self-hosting](self-hosting.md#what-sessions-get) has the details):
  - one for the primary repository and the others listed as read and
    write, with what the session's agent may do on GitHub: write contents,
    pull requests, issues and workflows for one that pushes; write pull
    requests and issues, and read contents, for one that comments; none
    for one that only reads. git and `gh` use it.
  - one that can only read (contents, pull requests, issues and CI
    results) the repositories listed as read only, or every repository of
    the account the GitHub App can reach when the project grants that
    (`owner/*`); for an agent that only reads, the others too. git uses it
    for those repositories.

  An agent that pushes can push to, open pull requests in, and change the
  workflows of every repository it can write to, so list as writable only
  the ones it needs. Sessions that don't need to push, such as review
  agents and planners, can comment or only read (`fctl run --github`), and
  a project can hold its agents to that (`fctl projects set --github`):
  their tokens can't push, whatever they read asks of them. An agent that
  comments can still close, edit and approve pull requests and issues.
  GitHub lets only tokens that can push resolve review threads, so the
  control plane resolves them for such an agent with a token it keeps to
  itself, for that one repository, after checking that the session may
  write there. Reading a whole account includes its private repositories, as far
  as the workspace may use them. Running sessions lose a repository removed
  from the project, or an account taken from its workspace, when their
  tokens are next refreshed, within the hour. A session's tokens are minted
  for the runner the session runs on only: a runner serves the workspaces
  its owner added it to, and gets no token for a session in another
  workspace.

  The project's ticket tracker narrows the issues permission
  ([Architecture](architecture.md#ticket-trackers)): tokens carry it as
  above where the project tracks its work in the GitHub issues of every
  repository it writes to, which is the default, and nowhere where it
  tracks it in Linear or nowhere. Where it is one repository's issues,
  only a write token for that repository alone carries it, since a token
  has one set of permissions for all its repositories. Public
  repositories' issues stay readable to anyone, so there the agent's
  instructions keep it away from them. A session that a maintainer of
  the project, who may change its tracker, starts with `fctl run
  --github-issues` gets the issues permission as for the GitHub issues
  of every repository it writes to, whatever the tracker says, for as
  long as it runs, whoever messages or resumes it.
- **Ticket trackers.** `fctl ticket` reaches the project's tracker through
  fctl-agent and the control plane, which holds the tracker's access: a
  Linear token reaches every team of its Linear workspace as fctl's app,
  so it never enters a sandbox, and for one repository's GitHub issues
  the control plane mints a token for that repository's issues alone for
  each call and revokes it after. Each call is kept to the project's
  choice, a Linear team (and project) or a repository, and a ticket
  outside it answers as one that doesn't exist, as do the parents,
  sub-tickets and search results outside it. Review agents reach no
  tracker. In an hour, a session's agent comments at most 30 times, files
  at most 20 tickets, changes at most 60 and reads at most 300 times, which
  also keeps one workspace's agents from using up the API of a Linear
  installation others share. What tickets and
  their comments say comes from whoever wrote them, inside the team or
  not: agents are told to take it as what the work is about, not as
  instructions, and do what the person who delegated it asks.

  Which accounts a workspace may use is its own admins' decision, and only
  within what they may touch on GitHub. A workspace's admin connects an
  account by signing in to GitHub with the instance's app, and the
  workspace gets what GitHub says that person may give: every repository of
  their own account, or of an organization whose every repository the app
  reaches and they administer all of; otherwise the repositories they
  administer. So no workspace reaches another account's repositories
  through the shared app, and an instance admin — who holds the app's key —
  connects nothing into a workspace, though they can take an account away.
  Installing the app on an account is necessary but not sufficient: it lets
  the instance mint tokens there, but no workspace uses it until one of its
  admins connects it with a GitHub sign-in
  ([ADR 0029](adr/0029-workspaces-connect-github-accounts-with-a-github-sign-in.md)).
  A connection is to the account, by GitHub's id, not its name: when a
  connected account is renamed or deleted and someone else takes its name,
  the workspace's sessions get no tokens there, and their pull requests no
  news from it, until its admins connect the account again.
- **Claude.** A short-lived access token for the session's owner. Refresh
  tokens never enter a sandbox: when the access token expires, Claude asks
  the control plane for a new one through its host. A session gets the
  token only while the person it runs for is an active member of its
  workspace and may still run it: start sessions in its project, or message
  it through a share. Once they leave the workspace, are removed or
  disabled, lose their role in the project or the share, or the
  workspace's two-factor requirement closes it to them, the sessions
  that run on their subscription stop, get no more Claude or GitHub tokens,
  and a message to one doesn't start it on their subscription again (nor
  on their runners, with the project's secrets); resuming it runs it on the
  resumer's. The token is in the sandbox, where the agent can read it, so
  whoever can message a session's agent (the project's editors, and people
  it is shared with to message) can have it hand over the token of the
  person the session runs as, as they can the session's GitHub tokens and
  the project's secrets: a full Claude login's token lasts hours, one from
  `claude setup-token` a year. Run sessions on a token from `claude
  setup-token` only where you would trust those people with it. Hosts
  replace every Claude token they know of with `[Claude token, redacted by
  fctl]` in what agents send to the control plane, so a token an agent
  prints by mistake (`printenv`), or repeats in its reply, stays out of the
  transcript and of what viewers see live; one it is asked to send
  elsewhere, or to write in another form (spaced out, encoded), doesn't.
- **claude.ai connectors.** A session on a full Claude login gets the
  connectors its owner has connected on claude.ai (mail, files, docs, chat),
  and the agent can use them with that person's access without asking, also
  when a repository or a web page tells it to. Long-lived tokens from
  `claude setup-token` don't reach connectors. The **claude.ai connectors**
  project setting turns them off for a project's sessions, and a
  repository's Claude settings can't turn them back on. New projects start with
  it off, in the app, from the CLI and through the API (turn it on in the
  form, with `fctl projects create --connectors`, or in Settings);
  projects from before the setting keep their connectors
  until a maintainer turns them off.
- **fctl.** No fctl credentials enter a sandbox. Through fctl-agent's
  session socket, anything the agent runs can rename its own session and
  describe it (`fctl edit`); in projects that publish screenshots, publish
  images at public links (`fctl publish`); and in projects that keep a
  memory or ask for retrospectives, read the project's notes, add notes,
  mark them stale or duplicates of other notes, and record, list and
  update retrospectives of its session (`fctl memory`, `fctl retro`); and
  where the agent may
  comment on GitHub, resolve review threads of its repositories' pull
  requests (`fctl resolve`); and nothing else. The control plane checks
  the values and the settings again. Turn publishing
  on only where anything the agents see may be shown to anyone with a
  link: text in a repository or on a web page can ask an agent to publish
  a screenshot, as it can ask it to send what it sees elsewhere.
- **Project memory** is text that the agent of every later session in the
  project starts with, and anything an agent runs can add to it, so text in
  a repository or on a web page that steers one agent can leave a note for
  the agents after it, past any review. `~/AGENTS.md` presents the notes
  as notes that don't override the person's request or fctl's rules; each
  note names the session that wrote it, the transcript shows it as it is
  kept, and the project's editors delete the ones they don't want, which
  the workspace's audit log records. Turn it off (`fctl projects set
  --project-memory off`) in projects whose agents read text from people
  you don't trust.
- **Schedules** start sessions that nobody watches start, on their
  owner's subscription: whoever may set one up may start sessions anyway,
  and a change by anyone else runs it on theirs. Runs that other tools ask
  for carry the tool's text (`context`), which anyone may have written,
  such as an issue's author: the agent gets it after the task, marked as
  information and not instructions, which makes it harder, not impossible,
  for that text to steer it. Give such a schedule the least GitHub access
  its task needs and a token that runs sessions in its project alone, and
  pause it to stop what tools start at once.
- **Review agents** run on their operator's Claude subscription, with
  their Claude access token in the sandbox like any session's, and read
  what pull requests say, which can steer them as any text can. Their
  GitHub token only reads, fctl posts what they find, and they get no
  secrets, setup script, memory or connectors. A pull request starts a
  review by itself only when its author may write to the repository, or
  one of fctl's agents or of the GitHub Apps the project's owners chose
  (Dependabot until they choose) opened it from a branch of the
  repository; anyone else's waits until someone in the project looks at
  it and asks for its review, since its text could otherwise have the
  agent hand the operator's token over
  ([ADR 0047](adr/0047-review-agents-review-outsiders-pull-requests-when-asked.md)).
  An app's pull requests can carry text from outside all the same, such
  as the release notes Dependabot and Renovate quote, and installing the
  dependencies they update runs their code, so their agent installs and
  runs nothing of them: add only apps whose pull requests you'd have it
  read unasked
  ([ADR 0054](adr/0054-review-agents-review-dependabots-pull-requests-by-themselves.md),
  [ADR 0057](adr/0057-projects-choose-which-apps-pull-requests-start-reviews.md)).
- **Project secrets** are exported to the agent's environment. Treat them as
  readable by anything the agent runs.
- **The image and the setup script.** Whoever sets a project's image or
  setup script decides what runs in its sandboxes, beside the credentials
  of the people who start its sessions: their Claude access token and the
  session's GitHub tokens. A full Claude login's access token lasts hours;
  a token from `claude setup-token` is valid for a year. Maintainers and
  owners set both, and for an image built from the repository, so does
  whoever can push to its default branch. Hosts' owners decide which images
  their machines run and which repositories they build images from
  ([below](#hosts)).
- **Package caches.** In a project that shares them (`fctl projects set
  --package-caches on`), its sessions on a runner share what their package
  managers download, in a Docker volume or, with Apple's `container`, a
  directory on the Mac that each sandbox mounts. Anything a session runs
  can change what they hold, and the project's later sessions on that
  runner build with it: package managers check a download against a
  lockfile's hashes as they fetch it, but most don't check again what is
  in their cache already, such as the sources Cargo unpacked. Projects
  never share caches, and review sessions, whose pull request could steer
  their agent, get none. Leave them off in projects whose agents run code
  from people you don't trust.
- **Attached files** are saved in the sandbox under `~/attachments/`, where
  anything the agent runs can read them, and Claude sees images in the
  message.

The `process` driver has no isolation at all and is for development only.

## Commits

Agents' commits are authored by the GitHub App's bot. They carry Claude's
`Co-authored-by` trailer unless the project turns off **Credit Claude in
commits**. They name the person who started the session only when that
person chose a commit email for the address they are in the workspace as
(Settings → Email addresses, or `fctl account set --commit-email`) and the
project credits people: a trailer then carries
their name and that address. Maintainers turn that off with **Credit people
in commits** in the project's settings (`fctl projects set --credit-people
off`); its sessions then don't receive the address at all. Everyone who can
read the repository sees the address in the commits, and
people who can see the session see it in the transcript; commits keep it
after it is removed or the account deleted. Settings and the API show the
address only to its owner, not to admins. fctl never puts sign-in emails in
commits.

## Billing

fctl is built so agents only ever use Claude **subscriptions**:

- `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `ANTHROPIC_BASE_URL`, the
  Bedrock/Vertex/Foundry switches and any other `ANTHROPIC_*` variables are
  removed from the sandbox environment, and project secrets cannot set them.
- A repository whose `.claude/settings.json` sets `apiKeyHelper` or
  `ANTHROPIC_*` variables runs without project settings.
- The session refuses to start if Claude reports any API key as its
  credential source.
- When Claude reports that a turn draws on paid extra usage, the agent is
  interrupted and the session pauses until the plan's limit resets.

## Hosts

Hosts connect outbound to the control plane over TLS (rustls with the Mozilla
root store); they open no ports. A host authenticates with a runner token
that its owner can replace, and that its owner and instance admins revoke by
disabling or removing the runner: whoever holds a runner's token runs the
sessions of the workspaces it serves, so only its owner gets a new one, not
an instance admin outside those workspaces, and only while none of those
workspaces' two-factor requirement closes it to them: a password alone
gets no runner token. Registering a runner for a workspace and replacing a
runner's token take confirming it's you, where the instance asks for
that, as adding one to a workspace does. Hosts receive a
session's credentials only while it runs, and keep nothing secret on disk
but their runner token (`~/.config/fctl/host-token`, mode 0600) and, when
their owner keeps it there, their owner's Claude token
(`~/.config/fctl/claude-token`, mode 0600).

**Claude tokens on runners.** Someone who would rather not give fctl their
Claude token keeps it on their runners (`fctl host claude set-token`) and
connects "On your runners"
([ADR 0050](adr/0050-claude-tokens-can-stay-on-their-owners-runners.md)).
The control plane is never given the token and stores none for them, hosts
tell it only whether they keep one and when it was saved, and they take
the token out of what sessions send wherever it appears whole: a breach of
D1 and the master key, or an operator of the instance, doesn't get it,
short of an agent that was made to pass it on (below). The control plane sends that
connection's sessions only to those of its owner's runners that keep a
token, in every workspace, whatever **Only on my runners** says, and each
of those hosts puts in the token it keeps, so no one else's machine ever
has it. Without one of those runners online, the sessions wait. What it
doesn't change: the agent has the token in its environment, as above, so
the people who may direct it can still have it handed over; and the
control plane decides what runs on a runner, so one taken over could start
a session there to the same end. The token is safe at rest and from other
people's machines, not from the sessions that run on it. Removing it from
a runner applies to the runs that start after, and doesn't revoke it: it
works until it expires, a year after `claude setup-token` made it.

A runner takes only the sessions of the workspaces it is added to, and so
sees their code, secrets and the tokens of the people who start them: the
session's GitHub tokens and the Claude token of the person it runs as. Its
owner adds it to a workspace for everyone's sessions there only as one of
its admins; a member adds theirs for their own sessions alone, which show
the machine nothing they couldn't read in the session, and a demoted
admin's runners run only their own from then on. A workspace's admins take
any runner out of it. Each member chooses, per workspace, to run their
sessions only on runners they registered (**Only on my runners**, `fctl
workspaces own-runners on`), so that no one else's machine sees their
Claude token or their sessions' work. Once a runner no longer runs a
session (taken out of the workspace, its owner left it or was disabled,
or the session's person keeps theirs on their own runners), the session
stops, and the runner gets no more tokens, messages or files for it,
whatever its host does. A Claude sign-in through a host, which passes the
full login's long-lived tokens through it, runs on one of the person's own
runners when one is online, else on a runner that runs their sessions.

**Updates.** Hosts update fctl by themselves to the version the control
plane runs, unless `auto_update = false` in `host.toml` (`fctl host
auto-update off`)
([ADR 0053](adr/0053-hosts-update-themselves-to-the-control-planes-version.md)).
The release comes from the host's own `download_base`, fctl.sh unless it
says otherwise: the control plane names only a version, which must be newer
than the host's, and never where to download it, so a control plane taken
over can't make its hosts install anything else, or an older fctl. The host
checks the archive against its SHA-256 and runs the new fctl before
anything changes, as `fctl self update` does. The checksum comes from the
same server, though: it catches broken downloads, not a release server
taken over. Leaving updates on trusts fctl.sh with what runs on the
machine, as installing from it does, without choosing when; with them off,
`fctl self update` updates the host when you choose, once you have looked
at the release. Turn updates off on machines where that matters.

Sessions run in their project's sandbox image or, without one, the host's:
by default `ghcr.io/hyperstructure/sandbox`, which the Image workflow of
the public [hyperstructure/sandbox](https://github.com/hyperstructure/sandbox)
repository builds, so anyone can read what goes into it. A host can use its
own build of it instead (`fctl host image build`) or a copy in another
registry (`image` in `host.toml`).

A project's own image runs only on hosts that allow it, and its build only
on hosts that build images from its repository: `allowed_images` and
`allowed_builds` in `host.toml`. Without `allowed_images`, a host runs any
image a project chooses, as hosts did before it, so set it on machines
where that matters. Without `allowed_builds`, it builds none. The control
plane schedules sessions by these lists, and the host checks them again
before it takes a session. To build, the host downloads the repository at
the tip of its default branch with the session's GitHub token, which the
build never sees, and runs the build in its container runtime's builder,
outside any sandbox ([Sandbox images](sandbox-images.md#which-images-a-host-runs)).
Hosts pull images with their own registry logins; fctl stores no registry
credentials.

## The control plane without Cloudflare

The standalone control plane ([Self-hosting](self-hosting.md#on-your-own-machine-or-cluster))
runs the same Worker, so the rest of this page holds for it too. What
Cloudflare does around the Worker, its server (`deploy/standalone`) does:

- **Addresses.** Sign-in rate limits and the audit log go by the address
  in `CF-Connecting-IP`. The server sets it from the connection, or behind
  `FCTL_TRUST_PROXY` proxies from what they added to `X-Forwarded-For`,
  and replaces any a client sends. It also drops headers that steer
  Miniflare (`MF-*`), which would take them from anyone, and answers
  `/cdn-cgi/`, where Miniflare has pages of its own, with a 404.
- **Secrets** are environment variables, or files (`<NAME>_FILE`).
  Without `FCTL_MASTER_KEY`, the server makes a master key and keeps it in
  its data directory, next to what it encrypts: a copy of the directory
  then has both. Set the key from a secret store to keep them apart.
- **TLS** is the reverse proxy's in front. Over plain http, anyone on the
  network in between reads sign-in cookies, CLI and runner tokens, and
  whatever the apps show; keep such an instance to a network you trust.
- **The data directory** holds D1 and every Durable Object as SQLite
  files: password hashes, encrypted credentials and secrets, transcripts
  and attached files. Encryption at rest is the disk's. One server at a
  time uses it (`server.lock`): two would each run the same sessions.
- Errors the Worker throws come back as plain 500s, without the stack
  traces Miniflare shows in development.

## Retention

- Deleting a project deletes its sessions with their transcripts (the event
  logs in their Durable Objects) and the files attached to their messages
  right away; an hourly job retries any that fail. Uploaded files that no
  message sent are deleted after a day.
- Expired browser sessions and tokens, password resets, sign-ins waiting
  for a second factor (ten minutes), device codes and invitations are
  deleted within the hour, and audit log entries after a year, as are
  passkey challenges (ten minutes). Turning two-factor off deletes the
  authenticator app's secret and the recovery codes; an admin's reset also
  deletes passkeys where they are the second factor required. A browser's notification subscription goes with the sign-in that
  made it (signing out, revoking or expiry), or when its push service says
  it is gone. A person's notifications wait in their Durable Object for at
  most a minute and are deleted once sent.
- Deleting a workspace deletes its projects, as deleting each would, and
  its audit log; the instance's audit log records who deleted it.
- People delete their own account in Settings (Danger zone) or with
  `fctl account delete`; admins delete members' accounts, and the primary
  instance owner admins' too. Both delete the account's addresses, sign-in methods, passkeys
  and two-factor sign-in, tokens, Claude connections, memberships, shares,
  follows and notification settings and browsers, and the workspaces only
  it is in, with their projects and sessions. Sessions and projects it
  created that others still use stay, credited to "Deleted user": the
  account remains only as a disabled placeholder, without its name,
  emails, commit emails or picture. Commits already pushed keep the
  trailers in them. The primary instance owner hands the role over
  before they can delete their account.
- Settings → Your data (or `fctl account export`) downloads everything the
  instance keeps about you, as JSON, with the sessions you started in the
  workspaces open to you: not those of workspaces you left. The notes and
  schedules you wrote, and the sessions you follow, come from the projects
  and sessions you still see, since others keep them up to date. Files you
  attached are listed in the transcripts there; each downloads from its
  session.
- A Linear account people link stays until they unlink it (Settings, or
  `fctl account linear disconnect`) or delete their fctl account. fctl's
  tokens in a Linear workspace stay while an fctl workspace is connected
  to it; once none is, the control plane revokes them at Linear and
  deletes them (within the hour if Linear doesn't answer), and it forgets
  them as soon as Linear says the app was uninstalled or refuses to renew
  them. A delegation waiting for someone to link their account keeps what
  Linear sent for an hour, and which Linear agent session reached which
  fctl session goes with the session.
- Hosts keep a stopped session's sandbox so it can resume, and remove it
  once it has been stopped for `keep_stopped_hours` (72 by default; 0 keeps
  them), or sooner when their disk has less free than `min_free_disk_gb`
  (20 GiB by default). Resuming after that starts a new sandbox on the
  session's branch, with everything that was pushed, and a new Claude
  conversation.
- Hosts keep the package caches of projects that share them, with what
  their sessions downloaded, until no session of the project has started
  there for `keep_caches_days` (14 by default; 0 keeps them). They empty
  them sooner once they take more than `max_cache_gb` (10 GiB by default),
  and when the disk runs low.

## Reporting a vulnerability

Report it privately through GitHub: **Security → Report a vulnerability** on
[hyperstructure/fctl](https://github.com/hyperstructure/fctl/security), with
details and a way to reproduce. Please do not open a public issue. We will
acknowledge within three working days and keep you updated until it is fixed.
