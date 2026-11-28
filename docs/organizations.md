# fctl for organizations

An organization that runs fctl for its people usually wants more than a
team does: fctl on its own machines or cloud account, accounts that follow
its directory, a portal or a chat bot of its own where people start
agents, and its other tools hearing what those agents do. This page shows
how the pieces fit, with an example of each, and links to the reference
for the details.

| You want to… | With | Read |
|---|---|---|
| Run fctl on your own machines, cluster or Cloudflare account, and keep it off the internet | A self-hosted instance | [Your own instance](#your-own-instance), [A private instance](self-hosting.md#a-private-instance) |
| Have your people sign in with your Google Workspace alone, with no fctl passwords | Domains that require a provider, on an instance of yours | [Signing in with your Google Workspace](#signing-in-with-your-google-workspace) |
| Create, change and close people's accounts from your identity provider or HR system, and let in no one else | Admin tokens, on an instance of yours | [People from your directory](#people-from-your-directory), [Accounts only from your directory](#accounts-only-from-your-directory) |
| Let your people start and steer agents from Slack, and hear from fctl there | fctl's Slack app | [Start agents from Slack](slack.md) |
| Let a portal, a chat bot or a form start agents for your people, each on their own Claude subscription | Service accounts | [Agents from your own portal or bot](#agents-from-your-own-portal-or-bot), and a guide to a bot of your own: [A Slack bot of your own](chat-bot.md) |
| Let CI, a pipeline or another tool start agents as one person | Access tokens and schedules | [Agents from CI and other tools](#agents-from-ci-and-other-tools) |
| Tell your chat, deploy pipeline or dashboards what agents do | Webhooks | [Your tools hear what agents do](#your-tools-hear-what-agents-do), and a guide for your warehouse: [Agents' work in your data warehouse](data-warehouse.md) |
| Let agents call your own services, and AWS, Google Cloud or Azure, without keys of yours | Identity tokens (OIDC) | [Agents in your own systems, with OIDC](#agents-in-your-own-systems-with-oidc) |
| Bring a fleet of machines online as runners without signing in on each | Enrollment tokens | [Runners for a fleet](#runners-for-a-fleet), and a guide for each cloud: [Runners in your cloud](runners-in-your-cloud.md) |
| Know who did what, and with which credential | Audit logs | [Oversight](#oversight) |

Whatever starts it, a session is a person's: it runs on their Claude
subscription, never on API billing, within what they may do in its
project, and it shows in their lists. A portal, a token or a schedule
only starts it.

## Your own instance

An instance is a control plane and the runners that connect to it.
app.fctl.sh is one, which hyperstructure runs. Yours runs in one container
on a machine or a Kubernetes cluster of yours, or as a Worker on your
Cloudflare account, with the same features ([Self-hosting](self-hosting.md)).
Some of what this page describes is only for an instance you run: admin
tokens, webhooks that reach your own network, and deciding for your people
whether service accounts act for them.

An organization's instance, behind a reverse proxy on the same machine
that serves `https://fctl.acme.dev`, with admin tokens on and webhooks
allowed to reach a network of its own:

```sh
docker run -d --name fctl --restart unless-stopped \
  -p 127.0.0.1:8787:8787 -v fctl-data:/data \
  -e FCTL_PUBLIC_URL=https://fctl.acme.dev \
  -e FCTL_INSTANCE_NAME="Acme factory" \
  -e FCTL_TRUST_PROXY=1 \
  -e FCTL_ADMIN_TOKENS=1 \
  -e FCTL_WEBHOOK_PRIVATE_HOSTS="10.20.0.0/16, *.corp.internal" \
  ghcr.io/hyperstructure/fctl-control
```

Set `FCTL_MASTER_KEY` from your secret store too, so that copies of the
data directory don't hold the key that encrypts what's in them
([Settings](self-hosting.md#settings)). To keep the instance on your own
network, with only the paths GitHub, Linear and the clouds call open to
the internet, see [A private instance](self-hosting.md#a-private-instance).

Once the first account exists, these are the settings an organization
usually changes, under **Admin → Access**, **Admin → Settings** and each
workspace's **Settings** tab, or with the CLI:

```sh
fctl admin allow '*@acme.dev'                  # everyone at acme.dev may sign up with Google and join the workspaces that invite them
fctl admin sign-in require acme.dev            # people at acme.dev sign in with your Google Workspace alone, no passwords
fctl admin set --self-sign-up off              # accounts come only from invitations and your directory's admin token
fctl admin set --require-two-factor on         # everyone who signs in with a password needs a second factor
fctl admin set --verify-sensitive-actions on   # making tokens, deleting projects… take a passkey or a code (the primary instance owner sets it)
fctl admin set --gravatar off                  # nobody's picture comes from gravatar.com
fctl admin set --service-account-consent off   # service accounts act for anyone in their workspace, without asking (below)
fctl admin set --members-create-workspaces off # only instance admins create workspaces: people work in the ones you put them in
fctl admin set --max-workspaces any            # each person can be in any number of workspaces (five by default)
fctl workspaces set --base-role editor -w acme # every member of acme starts sessions in every one of its projects
```

People sign in with a password, with passkeys and two-factor codes, or
with Google where the instance has
[Google sign-in](self-hosting.md#google-sign-in-optional), and the people
at your domains with your Google Workspace alone once you require it
([below](#signing-in-with-your-google-workspace)). fctl doesn't sign
people in through SAML or another OpenID Connect provider yet: people
whose accounts your directory makes sign in with Google where it hosts
their addresses, or with a password they choose from a reset link, as
below.

## Signing in with your Google Workspace

Where your people's addresses are your Google Workspace's, require Google
for your domains: they then sign in with it alone, under its two-step
verification and device checks, and can't sign in again once you close
their Google account. An instance admin does it under **Admin → Sign-in**,
or:

```sh
fctl admin sign-in require acme.dev acme.com
```

It first says whom it concerns and asks to confirm ("38 people sign in
with a password today: their next sign-in goes through Google, and their
passwords stop working. 4 people at other domains sign in as before.").
From then on, for everyone with an address at those domains:

- Their passwords, their passkeys alone (unless you add `--passkeys`) and
  their personal Google accounts stop signing them in, and saving the
  rule deletes the browser sessions, CLI sign-ins, access tokens and
  admin tokens they made that way, those from before the rule too.
- Their next Google sign-in, as their Workspace address, links Google to
  their fctl account and takes its password. The sign-in page turns its
  form into Google's button for an address there, and fctl emails them
  what changed, where the instance [sends
  email](self-hosting.md#email-optional).
- Their addresses there stay on their accounts: only an instance admin or
  your directory's admin token removes one.
- The primary instance owner keeps their password and passkeys, the way in
  while Google is down; an admin it would sign out signs in with Google
  first.

Contractors at other domains sign in as before, with the instance's
two-factor requirement. [Requiring Google for your
domains](self-hosting.md#requiring-google-for-your-domains) has the
details.

How long sign-ins last is yours to say too, so that someone whose Google
account you closed is out of fctl soon after, as their sign-ins end: a
browser's after 12 hours, say, a CLI's after 30 days, and access tokens'
after 90, which only admins make
([How long people stay signed in](self-hosting.md#how-long-people-stay-signed-in)):

```sh
fctl admin sign-in policies --browser-max 12h --cli-max 30d --token-max 90d --member-tokens off
```

And where your people work from known networks, sign-ins and requests can
come only from those: `fctl admin sign-in policies --networks
203.0.113.0/24,2001:db8::/32` ([Signing in only from your
networks](self-hosting.md#signing-in-only-from-your-networks)).

## People from your directory

On an instance of yours, your identity provider, your HR system or a
script creates people's accounts, keeps their names and addresses
current, puts them in their team's workspaces and projects, gives them
the company's Claude seat and closes their accounts as they leave, with
an admin token ([Provision people with admin
tokens](self-hosting.md#provision-people-with-admin-tokens)). app.fctl.sh
takes none.

1. **Turn them on**: `FCTL_ADMIN_TOKENS=1` on your own server, or
   `admin_tokens = true` in your Cloudflare instance's file, and deploy
   again.
2. **An instance admin makes one**, under **Admin → Settings → Admin
   tokens** or with `fctl admin tokens create "Okta provisioning" --days
   365`. It's shown once: keep it with your identity provider's secrets.
3. **Your program calls the people endpoints** with it
   ([The HTTP API](api.md#people-with-an-admin-token)), or runs the CLI
   with it as `FCTL_TOKEN`
   ([The CLI](cli.md#provisioning-people-with-an-admin-token)):

| When someone… | The CLI, with `FCTL_TOKEN=fctl_adm_…` | The API |
|---|---|---|
| joins | `fctl admin users create ada@acme.dev --given-name Ada --family-name Lovelace` | `POST /api/v1/admin/users` |
| joins a team | `fctl admin users add ada@acme.dev -w acme -p web:editor` | `PUT /api/v1/admin/users/{user}/workspaces/{workspace}` and `…/projects/{project}` |
| gets a Claude seat | `fctl admin users set ada@acme.dev --claude-token - --claude-label "Acme seat"`, with a token from `claude setup-token` on stdin | `PUT /api/v1/admin/users/{user}/claude` |
| changes name or address | `fctl admin users set ada@acme.dev --email ada.lovelace@acme.dev` | `PATCH /api/v1/admin/users/{user}` |
| leaves a team | `fctl admin users remove ada@acme.dev -p web -w acme` | `DELETE …/projects/{project}` and `…/workspaces/{workspace}` |
| leaves | `fctl admin disable ada@acme.dev`, later `fctl admin delete-user ada@acme.dev --confirm ada@acme.dev` | `PATCH /api/v1/admin/users/{user}` with `{"status": "disabled"}`, later `DELETE /api/v1/admin/users/{user}` |

A sync can run as often as you like: adding someone to a workspace or a
project they're in already gives them the role it names, and creating an
account for an address that has one answers `409` (`conflict`). fctl
doesn't speak SCIM: an identity provider reaches these endpoints through
its workflow automation, or a job of yours reads your directory and calls
them.

- **Signing in.** A new account has no password: its person signs in
  with Google where the instance has it and Google hosts their address
  (your Google Workspace, or Gmail), and that first sign-in links Google
  to their account. Otherwise they set a password with the link `fctl
  admin reset-link ada@acme.dev` makes (`POST
  /api/v1/admin/users/{user}/reset-link`), which works for an hour and
  which fctl emails them where the instance sends
  [email](self-hosting.md#email-optional). Once an
  account has a way in, a Google account links to it only from its
  Settings, as anyone's does, unless Google is
  [required for its domain](#signing-in-with-your-google-workspace): then
  Google, as their address there, is the only way in, and reset links
  aren't made for it.
- **Leaving.** Disabling an account signs its person out everywhere and
  stops the sessions that run on their subscription. Deleting it then
  works as deleting their own would: the workspaces only they are in go
  with it, and what others use stays.
- **What a token can't do.** It acts as the admin who made it, on the
  people endpoints alone: it makes nobody an instance admin, changes no
  invitations or settings, sees nothing inside projects but who is in
  them, and never changes the primary instance owner's account; only the
  primary owner's tokens change other admins'. It stops when it expires,
  someone revokes it, or its admin stops being an active admin.
- **Limits.** A person is in at most five workspaces, or as many as the
  instance's admins choose (`fctl admin set --max-workspaces`), the primary
  instance owner in any number. A workspace that requires two-factor
  sign-in stays closed to someone until they set it up.
- **Their Claude seat.** A long-lived token works until a year after
  `claude setup-token` made it, which fctl can't read from the token: say
  when with `--claude-made-at 2026-11-20` (`made_at`), or fctl counts the
  year from when you set it and says "around". Setting the same token
  again keeps its date, so a sync can set it on every run. Its person and
  their workspaces' admins hear before it runs out ([Claude
  seats](#claude-seats)).
- **Workspaces of their own.** People create workspaces of their own
  unless you keep that to your admins (`fctl admin set
  --members-create-workspaces off`, or **Admin → Settings → Workspaces**):
  then they work only in the workspaces your sync puts them in, and those
  they're invited to.

### Accounts only from your directory

Where your directory alone decides who has an account, nobody signs up
by themselves or makes workspaces of their own, everyone signs in through
your Google Workspace or with a second factor, and an account closes the
day its person leaves. An instance admin sets that up in about five
minutes, once your sync makes accounts with an admin token (above).

1. **Stop sign-ups**, so that signing in with Google through a domain's
   entry (`*@acme.dev`) makes no account:

   ```sh
   fctl admin set --self-sign-up off
   ```

   The domain's entry still admits the accounts at it; only invitations,
   addresses allowed one by one and your sync's admin token make new ones.
   The sign-in page tells someone without an account to ask an admin.
2. **Keep workspaces to your admins**, so that people work only in the
   workspaces your sync puts them in:

   ```sh
   fctl admin set --members-create-workspaces off
   ```

3. **Require your Google Workspace** where it hosts your people's
   addresses ([Signing in with your Google
   Workspace](#signing-in-with-your-google-workspace)):

   ```sh
   fctl admin sign-in require acme.dev
   ```

   Its own two-step verification then applies to their sign-ins. Ask
   for a code after Google too, where you want fctl's second factor as
   well (`fctl admin set --two-factor-google on --require-two-factor on`).
4. **Require a second factor of everyone else**, such as contractors at
   other domains who sign in with a password:

   ```sh
   fctl admin set --require-two-factor on
   ```

   `fctl admin users` then says who has it. Until someone sets one up,
   they can only do that, in the browser or with `fctl account two-factor
   enable`.
5. **Sign each person in for the first time.** Where Google hosts their
   address, Ada presses **Continue with Google**: that first sign-in links
   Google to the account your sync made. Otherwise, make Ada a link when
   Ada is about to start, as it works for an hour:

   ```sh
   fctl admin reset-link ada@contractor.io
   ```

   With email, fctl sends it to Ada and doesn't show it; without, it
   prints it for you to pass on. Ada chooses a password, then sets up a
   second factor (step 4).
6. **Close accounts the day people leave**, from the same sync:

   ```sh
   fctl admin disable ada@acme.dev
   ```

   That signs Ada out everywhere, and stops Ada's tokens and the
   sessions that run on Ada's Claude subscription. Where step 3 applies,
   closing Ada's Google account keeps Ada from signing in again, but the
   browsers and CLIs Ada is signed in with already go on until this
   does. Delete the account later (`fctl admin delete-user`), once nobody
   needs what only Ada has.

What can go wrong:

- **"Your account signs in with Google as your acme.dev address…"** Ada
  chose a personal Google account when Google asked: choose the Workspace
  one.
- **"Another Google account signs in to the account with this email…"**
  Ada's Workspace account was recreated, and the old one is still linked:
  `fctl admin unlink ada@acme.dev` (or **Admin → People → Unlink
  Google**) signs Ada out everywhere, and Ada's next sign-in links the new
  one. If the address went to someone else, disable the account instead
  (step 6): unlinking would give the new holder Ada's account.
- **"Google can't vouch for this address, as it doesn't host it…"**
  Ada's address isn't a Google Workspace or Gmail one, so Google doesn't
  sign Ada in: send a link (step 5).
- **"An account with this email exists. Sign in with your password, then
  link Google in Settings."** The account has a way in already, a
  password your sync set, say, and Google isn't required for its domain:
  Ada signs in that way, then links Google.
- **"Your organization makes the accounts here, and there's none for this
  address yet."** Someone your sync hasn't made an account for tried to
  sign up with Google. That's the setup working: their account comes from
  your directory.
- **"This would sign you out…"** You require Google of your own domain
  and signed in with a password: sign in with Google first, which shows
  it works for you, then require it.

fctl doesn't sign people in through SAML or an OpenID Connect provider
other than Google yet, nor take its SCIM: your sync calls the people
endpoints instead.

## Agents from your own portal or bot

A portal where your people pick a project and describe a task, a chat
bot, a form behind a script: each starts agents for someone else, and the
session should be that person's. A workspace's service account does that
([Service accounts](automation.md#service-accounts)). Each request names
the person it's for, and the session it starts is theirs: on their Claude
subscription, within what they may do in the project, in their lists,
and they hear when it finishes or asks something.

1. **Make one.** A workspace's admins make it on the workspace's
   **Service accounts** tab, or with `fctl service-accounts create "Acme
   portal" --description "Our agent portal at portal.acme.dev"`. Its
   secret (`fctl_svc_…`) is shown once: give it to your program alone. A
   workspace has at most ten.
2. **Choose whom it acts for.** By default it acts only for the members
   who allowed it, and fctl asks each of them on their Agent Floor. On an
   instance whose people all work for you, its admins can decide for
   them: turn off **Admin → Settings → Service accounts → People allow
   them first** (`fctl admin set --service-account-consent off`), and
   service accounts act for any member of their workspace. `fctl
   service-accounts people "Acme portal"` lists whom it may act for, and
   why not.
3. **Name the person in each request**, by their address or fctl id, in
   the `Fctl-On-Behalf-Of` header.

A portal's server, starting a session for the person signed in to it:

```js
const FCTL = "https://fctl.acme.dev/api/v1";

async function fctl(path, person, body) {
  const res = await fetch(`${FCTL}${path}`, {
    method: body ? "POST" : "GET",
    headers: {
      Authorization: `Bearer ${process.env.FCTL_SERVICE_ACCOUNT}`,
      "Fctl-On-Behalf-Of": person,
      "Content-Type": "application/json",
    },
    body: body && JSON.stringify(body),
  });
  const answer = await res.json();
  // A refusal's `message` says why, for the person: with `field: "on_behalf_of"` when
  // it can't act for them at all (not a member, not allowed, disabled, no two-factor),
  // as `forbidden` when they lack the role, such as editor to start sessions, and as
  // `not_found` for a project they can't see.
  if (!res.ok) throw Object.assign(new Error(answer.message), answer);
  return answer;
}

const session = await fctl("/projects/web/sessions", "ada@acme.dev", {
  prompt: "Fix the login redirect after a password reset",
});
// Keep session.id with Ada's request: webhooks' events name it. Its page in fctl
// is https://fctl.acme.dev/s/<session.id>, where Ada follows and steers it.
```

With the same header, `POST /sessions/{id}/messages` with `{"text": "Use
the staging database"}` answers the agent's question for Ada, and `GET
/sessions/{id}` reads how her session stands. From a shell, the CLI takes
the secret as `FCTL_TOKEN` and the person as `--on-behalf-of`: `fctl run
-p web --on-behalf-of ada@acme.dev -d "Fix the login redirect"`.

- **What else it does.** For the person it names, it messages,
  interrupts and stops their sessions and runs schedules, as an access
  token that runs sessions does. Without a person, it reads the
  workspace's projects, sessions and schedules, and runs a schedule for
  the person the schedule runs for. It changes no settings, people,
  secrets or tokens.
- **Following its sessions.** Service accounts open no live connections:
  read a transcript with `GET /api/v1/sessions/{id}/events`, or have a
  webhook post what happens, narrowed to service accounts' sessions
  ([below](#your-tools-hear-what-agents-do)).
- **Its secret's keeper.** A service account works while the admin who
  made its secret is an admin of the workspace. When they no longer are,
  it stops until another admin makes it a new secret (`fctl service-accounts
  secret "Acme portal"`), which you give your program; people's answers
  stay.
- **Where it shows.** Every session it starts says so, and for whom: on
  its page, in `fctl show`, in the workspace's audit log and in what
  webhooks post (`data.session.service_account`).

For Slack, fctl has an app of its own ([Start agents from Slack](slack.md)):
a workspace's admins connect their Slack workspace, and a mention of fctl,
a message to it or `/fctl` starts a session as whoever asked, whose card,
replies and questions come back to the thread; its agents read the thread
they were asked from, and only the public channels your admins make
readable. On an instance of yours, it's an app your organization creates
from a manifest ([Self-hosting](self-hosting.md#connect-slack-optional)),
and where your people sign in to Slack and to fctl through the same
provider, its admins can let workspaces know them by their address there
rather than have each link their Slack account.
[A Slack bot of your own](chat-bot.md) puts a service account together
for a bot of your own instead, for Microsoft Teams or a flow fctl's app
doesn't have: `@fctl` in a project's channel starts a session for
whoever wrote it, and the agent's questions, replies and pull requests
come back to the thread, where people answer with a click.

## Agents from CI and other tools

Which credential starts the work depends on whose session it should be:

| The session is… | Use |
|---|---|
| for the person a portal or a bot serves, a different one each time | [a service account](#agents-from-your-own-portal-or-bot) |
| the token's person's, with the task the tool sends | [an access token](automation.md#access-tokens) that runs sessions, limited to some projects and to what agents may do on GitHub |
| the token's person's, with a task you wrote and what the tool knows as information | a schedule that runs [when asked](automation.md#start-agents-from-other-tools), run with such a token |

```sh
fctl token "Release pipeline" --access run -p web --github comment --days 90
```

A tool's text, such as an issue filed in another tracker, comes from
people who can't instruct your agents: send it as a schedule's `context`,
which the agent gets marked as information for the task and not as
instructions. [Automation](automation.md) shows both, and how review
agents run from GitHub Actions with such a token.

## Your tools hear what agents do

A webhook posts what happens in a workspace to an address of yours as it
happens, signed: a session started, finished a turn, asked something, has
its pull requests ready for review or failed; an agent's pull request
opened or merged; a review agent's review posted; a schedule's run that
started nothing ([Webhooks](automation.md#webhooks)). A workspace's
admins add them for all its projects or some, on its **Webhooks** tab or
with `fctl webhooks add`; a project's owners and maintainers add its own,
under its **Settings → Webhooks**.

| To tell… | Events | Narrowed with |
|---|---|---|
| a team's chat that an agent needs someone | `session.question`, `session.failed` | `--projects web,api` |
| reviewers that pull requests are ready | `session.ready_for_review` | |
| your portal how the sessions it started go | `session.*` | `--started-by service-accounts` |
| a deploy pipeline that a scheduled session finished | `session.finished` | `--started-by schedules` |
| a dashboard of reviews and their findings | `review.finished` | |

```sh
fctl webhooks add https://portal.acme.dev/fctl -w acme --name "Portal" \
  --events session.finished,session.question,session.ready_for_review,session.failed \
  --started-by service-accounts
```

Before the first event, fctl posts the address a challenge that your
receiver answers ([Checking the address](automation.md#checking-the-address)),
then shows the webhook's signing secret (`whsec_…`) once. Requests are
signed as [Standard Webhooks](https://www.standardwebhooks.com) sign
them, so its libraries check one in a call:

```js
import { Webhook } from "standardwebhooks";

export async function onFctl(req) {
  const body = await req.text(); // as it came: the signature covers these bytes
  const sent = JSON.parse(body);
  if (sent.type === "url_verification") return new Response(sent.challenge);
  const event = new Webhook(process.env.FCTL_WEBHOOK_SECRET).verify(body, Object.fromEntries(req.headers));
  if (event.workspace.id !== process.env.FCTL_WORKSPACE_ID) return new Response("Not ours", { status: 400 });
  await enqueue(event); // answer within 10 seconds, and do the work after
  return new Response(null, { status: 204 });
}
```

A request that fails goes again 7 times over about 21 hours, and each
webhook keeps what went out in the last 30 days, to read and resend
([When a request fails](automation.md#when-a-request-fails)). Webhooks
post to `https://` addresses on the public internet; an instance of yours
can let them reach receivers on your own network, and post over `http://`
([Webhooks to your own network](self-hosting.md#webhooks-to-your-own-network)).
A webhook posts while whoever chose its address still manages it: when
they leave the workspace or lose the role, fctl turns it off and says
why, and the admin who turns it on again keeps it from then on.

[Agents' work in your data warehouse](data-warehouse.md) keeps every
event in BigQuery or Snowflake, with the queries for sessions a day,
time to review and merged pull requests.

## Agents in your own systems, with OIDC

An agent proves to your services and clouds that a request comes from one
of your workspace's sessions with an identity token: an OpenID Connect
(OIDC) ID token, signed by fctl, that says whom the session runs for, its
project, runner and what it may do on GitHub, and lasts 10 minutes
([Identity tokens](identity-tokens.md)). Your workspace is an OIDC issuer
of its own, so a cloud set up to trust its agents trusts no one else's:

| To let agents… | Set up | And trust |
|---|---|---|
| call an internal API | nothing in fctl: the API checks tokens with your issuer's keys | `iss`, `aud`, then `project_id`, `user_id`, `github_access` |
| use AWS | the role, in the project (`--aws-role`) | the trust policy `fctl projects identity` prints |
| use Google Cloud | the workload identity provider (`--gcp-provider`) | the pool's provider and the service account's binding it prints |
| use Azure | the application (`--azure-tenant`, `--azure-client`) | a federated credential for `project:prj_…` |
| get secrets from Vault | nothing in fctl | its JWT auth method, bound to your issuer and claims |

```sh
fctl projects set -p web --aws-role arn:aws:iam::123456789012:role/fctl-web
fctl projects identity -p web         # the provider, the trust policy and the commands, filled in
```

The project's agents then run `aws`, `gcloud` and the clouds' SDKs as they
are, with credentials that last an hour and come again, and nothing to
leak but a token that expires in minutes. For the rest, `fctl id-token`
mints a token for whichever service an agent calls:

```sh
curl -H "Authorization: Bearer $(fctl id-token -a https://deploys.acme.dev)" https://deploys.acme.dev/api/…
```

Whatever runs in a session's sandbox can mint its tokens, and so can the
owner of the runner it runs on: trust a project's agents with what they
should do there, and use `github_access`, `started_by` and
`runner_personal` to leave out sessions that read text from outsiders or
run on members' own machines ([What a token proves](identity-tokens.md#what-a-token-proves)).

## Runners for a fleet

Machines that your autoscaling creates, or that you set up without
signing in on them, enroll themselves as a workspace's runners with one
of its enrollment tokens
([Enrollment tokens](cli.md#enrollment-tokens)):

```sh
fctl runners enrollment-tokens create "Build fleet" -w acme --max-runners 20 --remove-offline-after 15m

# in each machine's boot script
curl -fsSL https://fctl.sh/install | sh
FCTL_SERVER=https://fctl.acme.dev FCTL_ENROLL_TOKEN=fctl_et_… fctl host install
```

Their sessions start from fctl's sandbox image, a project's own, or one
built from its repository; a host's `allowed_images` and `allowed_builds`
hold it to the images you choose
([Which images a host runs](sandbox-images.md#which-images-a-host-runs)).

[Runners in your cloud](runners-in-your-cloud.md) builds such a fleet on
AWS, Google Cloud or Azure, step by step: a machine image with Docker,
fctl and the sandbox image ready, a group that grows and shrinks, and a
hook that lets each machine's agents finish their turns before the cloud
takes it.

Each token's runners are a pool. Give it labels, and keep one pool of
large machines for the projects that ask for them and one in another
region for others; let a pool that grows and shrinks pack sessions onto
its busiest runners, so that idle ones can go; and have your autoscaler
scale it to what it needs now, which it reads with the enrollment token,
a service account, `fctl runners demand --cloudwatch` or, on your own
instance, Prometheus ([Runner pools](cli.md#runner-pools)):

```sh
fctl runners enrollment-tokens create "AWS large" -w acme --labels aws,large --max-runners 20 --remove-offline-after 15m
fctl projects set -p ml --runners large
```

## Claude seats

Sessions run on each person's own Claude subscription: give your people
Team or Enterprise seats, which Claude's organization admins assign
([Manage members on Team and Enterprise
plans](https://support.claude.com/en/articles/13133750-manage-members-on-team-and-enterprise-plans)), and
each connects theirs in **Settings → Claude**, or your sync sets it for
them ([above](#people-from-your-directory)). A workspace's admins then see
on its **People** tab whether everyone can run sessions this week and
next: who has Claude connected for it and on which plan, whose
connection Claude stopped taking, whose long-lived token runs out within a
month, and whose sessions keep pausing on usage limits, with what to do
about each. `fctl workspaces members -w acme --claude --problems` says the
same. They never see a token, the Claude account it belongs to or the
label its person gave it, and each person reads in the workspace's
**Settings → You** what its admins see of theirs.

```
$ fctl workspaces members -w acme --claude
NAME           CLAUDE                      PLAN   ANSWERED   RUNS OUT        LIMIT PAUSES (7D)
Ada Lovelace   login                       Max    5m ago     -               0
Grace Hopper   token, set by admin token   Team   2h ago     ~Dec 14 (21d)   6
Linus Lee      none                        -      -          -               -
1 person has no Claude connection for Acme: their sessions here can't start.
```

How much admins see, and how they hear of it:

| Setting | Where | Choices |
|---|---|---|
| The most a workspace's admins may see | **Admin → Settings**, `fctl admin set --claude-oversight` | `details` (the default): the connection's kind and who set it, its plan, when Claude last answered it, when it runs out, and whether sessions wait for a limit to reset; `readiness`: only whether each member's sessions can run and when their token runs out; `off`: nothing |
| Whether they may see limit pauses | **Admin → Settings**, `fctl admin set --claude-limit-pauses` | on (the default): how often each member's sessions in the workspace paused on usage limits in 7 and 30 days |
| Instance admins see everyone's | **Admin → Settings**, `fctl admin set --claude-instance-view` | off (the default); on, **Admin → People** and `fctl admin claude` show every account's connection, as workspace admins see their members' |
| When fctl marks a token ended | **Admin → Settings**, `fctl admin set --claude-refusals` | after Claude refused it in 2 sessions since it last answered it (the default), 1 to 5 |
| What a workspace's admins see | its **Settings → Claude**, `fctl workspaces set --claude-oversight`, `--claude-limit-pauses` | within what the instance lets them |
| How its admins hear by email | its **Settings → Claude**, `fctl workspaces set --claude-admin-email` | `weekly` (the default): one email a week while a member's token runs out within a month or stopped working; `each`: as each member hears; `off` |

- **When people hear.** fctl tells each person 30 days and 7 days before
  their long-lived token runs out, and as soon as Claude stops taking
  their connection: by email where the instance sends
  [email](self-hosting.md#email-optional), and in the app, under the
  connection in **Settings → Claude** and, in a token's last week, on the
  Agent Floor. A login Claude ends is told the same way; logins don't run
  out on a date, as Claude renews them.
- **A token that stopped working.** Claude refuses a revoked or expired
  token in each session that runs on it; once it did in as many sessions
  as the instance says since it last answered it, fctl marks it ended and
  sessions run on the person's other connection, if one works. A new token
  from `claude setup-token`, connected in **Settings → Claude** or set
  with an admin token, is the way back.
- **What fctl keeps for it.** When Claude last answered each connection,
  when its token runs out, and each session's pauses on usage limits and
  each refusal for 30 days
  ([Retention](security.md#retention)).

## Oversight

- **Audit logs.** Each workspace's (its admins: its **Audit log** tab,
  `fctl workspaces audit`) records what happens to its people, roles,
  runners, settings, projects and sessions; the instance's (**Admin →
  Audit log**, `fctl admin audit`) the rest. They record what an admin
  token did, as its admin and with which token, and what a service
  account did, for whom. Entries are kept for a year.
- **Who started what.** Each session names the person it runs for and
  how it started: by them or a token of theirs, a schedule, an
  autopilot, a ticket or a service account. Its page and `fctl show` say
  so, and webhooks' `data.session.started_by`.
- **What fctl keeps.** How long each kind of data stays, and what
  deleting a project, a workspace or an account removes:
  [Retention](security.md#retention). Each person can export their data;
  **Admin → Settings → Data exports** sets how often.
- **Claude connections.** Whether each member can run sessions, on
  which plan, and whose token runs out soon, as much as the instance and
  the workspace let admins see: [Claude seats](#claude-seats).
- **What each credential reaches**, and how fctl keeps the rest from it:
  [Access](security.md#access).

## Credentials at a glance

| Credential | Who makes it | Acts as | Reaches | Stops working when |
|---|---|---|---|---|
| Access token, `fctl_pat_…` | anyone, for themselves | its person | their projects and sessions, as limited when made | it expires, they revoke it, or their account is disabled |
| Service account, `fctl_svc_…` | a workspace's admins | the person each request names | that person's sessions in its workspace's projects, as a token that runs sessions; without a person, reading | it's deleted or gets a new secret, or the admin who made its secret is no longer an admin of the workspace (or has to set up two-factor sign-in first) |
| Admin token, `fctl_adm_…` | an instance admin, where the instance takes them | its admin | the people endpoints of the admin API | it expires, someone revokes it, its admin is no longer an active one, or `FCTL_ADMIN_TOKENS` is off |
| Enrollment token, `fctl_et_…` | a workspace's admins, or a member for their own sessions | nobody | enrolling machines as the workspace's runners | it expires (90 days unless set) or someone revokes it; its runners stay |
| Webhook secret, `whsec_…` | made with the webhook | nobody | signing what fctl posts to it | a day after it's rotated, or when the webhook is deleted; the webhook stops posting once whoever chose its address no longer manages it |
| Identity token (OIDC), a JWT | an agent in a session's sandbox (`fctl id-token`), or fctl-agent for its project's clouds | nobody: it says which session, person, project and runner it comes from | nothing in fctl; whatever trusts your workspace's issuer, for its audience | 10 minutes after it was signed |
