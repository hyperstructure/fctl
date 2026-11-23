# fctl for organizations

An organization that runs fctl for its people usually wants more than a
team does: fctl on its own machines or cloud account, accounts that follow
its directory, a portal or a chat bot of its own where people start
agents, and its other tools hearing what those agents do. This page shows
how the pieces fit, with an example of each, and links to the reference
for the details.

| You want to… | With | Read |
|---|---|---|
| Run fctl on your own machines, cluster or Cloudflare account | A self-hosted instance | [Your own instance](#your-own-instance) |
| Create, change and close people's accounts from your identity provider or HR system | Admin tokens, on an instance of yours | [People from your directory](#people-from-your-directory) |
| Let a portal, a chat bot or a form start agents for your people, each on their own Claude subscription | Service accounts | [Agents from your own portal or bot](#agents-from-your-own-portal-or-bot) |
| Let CI, a pipeline or another tool start agents as one person | Access tokens and schedules | [Agents from CI and other tools](#agents-from-ci-and-other-tools) |
| Tell your chat, deploy pipeline or dashboards what agents do | Webhooks | [Your tools hear what agents do](#your-tools-hear-what-agents-do) |
| Let agents call your own services, and AWS, Google Cloud or Azure, without keys of yours | Identity tokens (OIDC) | [Agents in your own systems, with OIDC](#agents-in-your-own-systems-with-oidc) |
| Bring a fleet of machines online as runners without signing in on each | Enrollment tokens | [Runners for a fleet](#runners-for-a-fleet) |
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
([Settings](self-hosting.md#settings)).

Once the first account exists, these are the settings an organization
usually changes, under **Admin → Access**, **Admin → Settings** and each
workspace's **Settings** tab, or with the CLI:

```sh
fctl admin allow '*@acme.dev'                  # everyone at acme.dev may sign up with Google and join the workspaces that invite them
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
[Google sign-in](self-hosting.md#google-sign-in-optional). fctl doesn't
sign people in through SAML or another OpenID Connect provider: people
whose accounts your directory makes sign in with Google where it hosts
their addresses, or with a password they choose from a reset link, as
below.

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
  /api/v1/admin/users/{user}/reset-link`), which fctl emails them where
  the instance sends [email](self-hosting.md#email-optional). Once an
  account has a way in, a Google account links to it only from its
  Settings, as anyone's does.
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
- **Workspaces of their own.** People create workspaces of their own
  unless you keep that to your admins (`fctl admin set
  --members-create-workspaces off`, or **Admin → Settings → Workspaces**):
  then they work only in the workspaces your sync puts them in, and those
  they're invited to.

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
