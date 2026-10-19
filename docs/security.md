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
| Claude credentials | encrypted (AES-256-GCM) in D1 | sessions their owner starts |
| Project secrets | encrypted in D1 | that project's sessions, as environment variables |
| Workspace avatars | images in D1 | the workspace's members, people it invited, instance admins |
| Attached files | the project's Durable Object | people who can see the session they were sent in; its runner and agent |
| Published screenshots | the project's Durable Object, found by the SHA-256 of their links' tokens in D1 | anyone with a link, until it expires |
| GitHub App key | a Worker secret | the control plane, to mint short-lived tokens |
| Web Push (VAPID) key | encrypted in D1 | the control plane, to sign notifications |
| Master key | a Worker secret | the control plane |

Encrypted values are bound to their row (the AES-GCM associated data names the
owner and purpose), so a value copied between rows does not decrypt.

## Access

- Accounts exist only for allowlisted emails or domains. Removing an
  allowlist entry disables the accounts it admitted and revokes their tokens.
  Each of an account's addresses was invited (or admitted) on its own, and
  any of them signs in to the same account, with the same factors.
- **Workspaces** keep their projects, sessions, files and secrets to their
  members: people outside one are told its projects and sessions don't
  exist, whatever they started there or was shared with them, and instance
  admins see inside only the ones they are members of. A workspace sees
  only the address each member joined as.
- Browser sessions are `__Host-` cookies (`Secure`, `HttpOnly`,
  `SameSite=Lax`). Requests that change state must come from the app's own
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
  token, always). Admins can require it:
  accounts that sign in with a password, and with Google sign-ins asking
  too every account, then can do nothing until they set it up, in the
  browser or with their CLI and access tokens. An attacker who has the
  password of an account that hasn't set it up yet could set it up first;
  the email about it (when email is set up) and the audit log show that,
  and admins reset two-factor for people who lost their phone. A
  workspace's admins can require it of its members in the same way: a
  member without a second factor can't open the workspace until they set
  one up, and their sign-ins ask for it.
- **Passkeys** sign in without a password or code: the device checks it's
  the person (fingerprint, face, screen lock or PIN), and the signature
  works only for this instance's host name, so a fake site can't use it.
  fctl keeps the public key, each challenge works once for ten minutes, and
  a counter that goes back (a copied passkey's would) refuses the sign-in.
  Adding one takes the password unless the browser signed in within 15
  minutes, so a stolen session can't add a way in, and emails the person.
  Where two-factor is required, a passkey also counts as the second factor.
- The CLI signs in with a device flow (you approve a code in the browser) and
  gets a revocable token; Settings lists every device and token.
- Sign-in, signup and reset endpoints are rate limited per address and per
  account; unknown emails take as long to fail as known ones.
- The web app ships a strict Content-Security-Policy (`script-src 'self'`, no
  inline scripts), `frame-ancestors 'none'`, HSTS, and renders agent output
  as text (Markdown is parsed into elements, never injected as HTML).
- Notifications go through each browser's push service (Google's for
  Chrome, Mozilla's for Firefox, Apple's for Safari, Microsoft's for Edge),
  encrypted for that browser alone (RFC 8291): the service sees when one
  goes and how big it is, not what it says. They name the session and
  quote the start of the agent's last message, and show like any
  notification, on a lock screen too; people choose what notifies them in
  Settings. The control plane only sends to `https://` addresses of named
  hosts, and to at most 20 browsers per person.
- File uploads are `application/octet-stream` bodies, which browsers cannot
  send cross-site without asking first, and pass the same origin check.
  Attached files are served with `nosniff` and a sandboxing
  Content-Security-Policy; only PNG, JPEG, GIF and WebP display in place,
  everything else downloads, so an uploaded HTML or SVG file never runs as
  part of the app.
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

- **Network.** Sandboxes have outbound network access (to install
  dependencies and reach GitHub and Claude). Do not run hosts on networks
  where an outbound connection from an untrusted process is a problem.
- **GitHub.** Installation tokens for the project's repositories and no
  others, within the GitHub accounts and repositories its workspace may use,
  valid for an hour and refreshed on demand
  ([self-hosting](self-hosting.md#what-sessions-get) has the details):
  - one that can write (contents, pull requests, issues and workflows) to
    the primary repository and the others listed as read and write. git
    and `gh` use it.
  - one that can only read (contents, pull requests, issues and CI
    results) the repositories listed as read only, or every repository of
    the account the GitHub App can reach when the project grants that
    (`owner/*`). git uses it for those repositories.

  An agent can push to, open pull requests in, and change the workflows of
  every repository it can write to, so list as writable only the ones it
  needs. Reading a whole account includes its private repositories, as far
  as the workspace may use them. Running sessions lose a repository removed
  from the project, or an account taken from its workspace, when their
  tokens are next refreshed, within the hour. Which workspaces use which
  accounts is an instance admin's decision: anyone can install the app on
  an account they own, but only an instance admin's approval lets a
  workspace use it, since GitHub's setup redirect can name anyone's
  installation.
- **Claude.** A short-lived access token for the session's owner. Refresh
  tokens never enter a sandbox: when the access token expires, Claude asks
  the control plane for a new one through its host.
- **claude.ai connectors.** A session on a full Claude login gets the
  connectors its owner has connected on claude.ai (mail, files, docs, chat),
  and the agent can use them with that person's access without asking, also
  when a repository or a web page tells it to. Long-lived tokens from
  `claude setup-token` don't reach connectors. The **claude.ai connectors**
  project setting turns them off for a project's sessions, and a
  repository's Claude settings can't turn them back on. Projects created in
  the app start with it off (from the CLI: `fctl projects create
  --no-connectors`); projects from before the setting keep their connectors
  until a maintainer turns them off.
- **fctl.** No fctl credentials enter a sandbox. Through fctl-agent's
  session socket, anything the agent runs can rename its own session and
  describe it (`fctl edit`) and, in projects that publish screenshots,
  publish images at public links (`fctl publish`), and nothing else; the
  control plane checks the values and the settings again. Turn publishing
  on only where anything the agents see may be shown to anyone with a
  link: text in a repository or on a web page can ask an agent to publish
  a screenshot, as it can ask it to send what it sees elsewhere.
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
root store); they open no ports. A host authenticates with a runner token that
its owner and instance admins can rotate or revoke. Hosts receive a
session's credentials only while it runs, and keep nothing secret on disk
but their runner token (`~/.config/fctl/host-token`, mode 0600).

A runner takes only the sessions of the workspaces it is added to, and so
sees their code, secrets and the tokens of the people who start them. Its
owner adds it only to workspaces they are an admin of, and a workspace's
admins take any runner out of it. A Claude sign-in through a host runs only
on a runner of the person's workspaces, or one they registered.

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
  admin admins' too. Both delete the account's addresses, sign-in methods, passkeys
  and two-factor sign-in, tokens, Claude connections, memberships, shares,
  follows and notification settings and browsers, and the workspaces only
  it is in, with their projects and sessions. Sessions and projects it
  created that others still use stay, credited to "Deleted user": the
  account remains only as a disabled placeholder, without its name,
  emails, commit emails or picture. Commits already pushed keep the
  trailers in them. The instance's primary admin hands the role over
  before they can delete their account.
- Settings → Your data (or `fctl account export`) downloads everything the
  instance keeps about you, as JSON. Files you attached are listed in the
  transcripts there; each downloads from its session.
- Hosts keep a stopped session's sandbox so it can resume, and remove it
  once it has been stopped for `keep_stopped_hours` (72 by default; 0 keeps
  them), or sooner when their disk has less free than `min_free_disk_gb`
  (20 GiB by default). Resuming after that starts a new sandbox on the
  session's branch, with everything that was pushed, and a new Claude
  conversation.

## Reporting a vulnerability

Report it privately through GitHub: **Security → Report a vulnerability** on
[hyperstructure/fctl](https://github.com/hyperstructure/fctl/security), with
details and a way to reproduce. Please do not open a public issue. We will
acknowledge within three working days and keep you updated until it is fixed.
