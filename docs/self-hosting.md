# Self-hosting

An fctl instance is a control plane (the API and the web app, with the
Durable Objects and the D1 database that hold its state) and the hosts on
your machines that connect to it. Nothing in the code is tied to fctl.sh:
everything instance-specific is configuration. The control plane runs in one
of two ways, with the same features, settings, hosts and apps:

| | On your own machine or cluster | On Cloudflare |
|---|---|---|
| What runs | one container, `ghcr.io/hyperstructure/fctl-control`: the Worker on workerd, Cloudflare's open-source Workers runtime | one Worker with Durable Objects and a D1 database, on your account |
| What you need | a machine with Docker, or a Kubernetes cluster; no account anywhere | a Cloudflare account with a domain on it |
| HTTPS | a reverse proxy of your choice, or plain http on a network you trust | Cloudflare's, on your domain |
| Backups | copies of its data directory | D1's 30 days of point-in-time history |
| Made for | trying fctl out, a spare machine, a homelab, a cluster you run | a team's everyday instance, as app.fctl.sh |

Set up the control plane one of the two ways, then the rest of this page
(your team, GitHub, Google sign-in, email, hosts) is the same for both.

## On your own machine or cluster

The image runs the Worker and the web app that run on Cloudflare, on
workerd through Miniflare, with all its state (the database, every
session's event log, attached files) in `/data`
([ADR 0026](adr/0026-the-control-plane-runs-without-cloudflare-on-workerd.md)).
It listens on port 8787, for amd64 and arm64 machines alike (a Raspberry Pi
4 or later will do), and each release of fctl publishes it.

### With Docker

```sh
docker run -d --name fctl --restart unless-stopped \
  -p 8787:8787 -v fctl-data:/data \
  -e FCTL_PUBLIC_URL=http://192.168.1.20:8787 \
  ghcr.io/hyperstructure/fctl-control
docker logs fctl
```

`FCTL_PUBLIC_URL` is the address people open the app at and hosts connect
to: the machine's address on your network here, `http://localhost:8787`
to keep it to this machine, or the https address of a
[reverse proxy](#https-and-reverse-proxies) in front. The log has the link
that creates the first account, the primary instance owner's:

```
fctl control plane 0.11.0 at http://192.168.1.20:8787, listening on 0.0.0.0:8787, data in /data
Create the primary instance owner's account (the link works once):
  http://192.168.1.20:8787/setup#token=…
```

Open it, then carry on with [your team](#the-first-account-and-your-team).
[`deploy/standalone/compose.yaml`](../deploy/standalone/compose.yaml) does
the same with Docker Compose. To build the image yourself, in a clone:
`docker build -f deploy/standalone/Dockerfile -t fctl-control .`

### On Kubernetes

[`deploy/standalone/kubernetes.yaml`](../deploy/standalone/kubernetes.yaml)
has a StatefulSet of one, with a volume for its data, a Service and an
Ingress. Set `FCTL_PUBLIC_URL` and the Ingress's host in it to your address,
then:

```sh
kubectl create namespace fctl
kubectl -n fctl create secret generic fctl \
  --from-literal=FCTL_MASTER_KEY="$(openssl rand -base64 32)"
kubectl -n fctl apply -f kubernetes.yaml
kubectl -n fctl logs statefulset/fctl-control
```

Keep it to one replica. Its state is SQLite files on the volume, and two
servers on them would each run the same sessions: a second one waits for
the first to stop, and gives up after 45 seconds. The Ingress has to pass
WebSockets, as ingress-nginx, Traefik and the cloud providers' do, and
bodies of up to 25 MB, for attachments. The pod reads the `fctl` Secret as
environment variables: add the [GitHub App's](#connect-github-optional-recommended),
[Google's](#google-sign-in-optional) and [Resend's](#email-optional)
secrets there when you use them.

### From a clone, without Docker

With Rust and Node.js 22 or newer, as for [development](development.md#setup):

```sh
git clone https://github.com/hyperstructure/fctl && cd fctl
cargo xtask serve --public-url http://192.168.1.20:8787
```

It builds the Worker and the web app, then runs the server the image runs,
with its state in `.fctl-serve/` (`--data` for another directory). It
listens on every address unless the URL is on this machine.

### Settings

The server takes the settings the Worker takes on Cloudflare, as
environment variables. Each can come from a file instead, `<NAME>_FILE`, as
Docker and Kubernetes secrets do.

| Variable | Default | What it is |
|---|---|---|
| `FCTL_PUBLIC_URL` | `http://localhost:8787` | The address people open the app at and hosts connect to |
| `FCTL_INSTANCE_NAME` | `fctl` | Shown in the app and in emails |
| `FCTL_MASTER_KEY` | made in the data directory | 32 random bytes in base64 (`openssl rand -base64 32`) that encrypt stored credentials and secrets |
| `FCTL_SETUP_TOKEN` | made at each start | The token in the setup link, which works until the first account exists |
| `FCTL_DATA_DIR` | `/data` in the image | Where the state lives |
| `FCTL_PORT`, `FCTL_LISTEN` | `8787` (or `PORT`), `0.0.0.0` | Where it listens |
| `FCTL_TRUST_PROXY` | `0` | How many [reverse proxies](#https-and-reverse-proxies) in front add `X-Forwarded-For` |
| `GITHUB_APP_ID`, `GITHUB_APP_SLUG`, `GITHUB_CLIENT_ID`, `GITHUB_DEFAULT_OWNER`, `GITHUB_APP_PRIVATE_KEY` | | [The GitHub App](#connect-github-optional-recommended) |
| `GOOGLE_CLIENT_ID`, `GOOGLE_CLIENT_SECRET` | | [Google sign-in](#google-sign-in-optional) |
| `RESEND_API_KEY`, `FCTL_EMAIL_FROM` | | [Email](#email-optional) |

Without `FCTL_MASTER_KEY`, the server makes a master key the first time it
starts and keeps it in `master-key` in the data directory, next to what it
encrypts, so a copy of the directory has both. To keep them apart, as the
Kubernetes example does, set it from a secret. Once the server has made
one, set that one: replacing the key makes every stored Claude credential
and project secret unreadable, so the server refuses another.

### HTTPS and reverse proxies

Plain http works on a network you trust. Over it, though, anyone on the
network in between reads sign-ins and tokens, and browsers offer passkeys,
notifications and installing the app only over https (or on `localhost`).
For those, put a reverse proxy with TLS in front, which passes WebSockets
and bodies of up to 25 MB, and set `FCTL_PUBLIC_URL` to its https address.
[Caddy](https://caddyserver.com), for one, gets the certificate itself:

```
fctl.example.com {
	reverse_proxy localhost:8787
}
```

On a [Tailscale](https://tailscale.com) network,
`tailscale serve --bg 8787` gives the machine an https address
(`https://<machine>.<tailnet>.ts.net`) without making it public.

Behind a proxy, set `FCTL_TRUST_PROXY=1` (`2` behind two of them, and so
on). The control plane limits sign-in attempts and records addresses in
the audit log by the address each request comes from, which a proxy puts
in the `X-Forwarded-For` header it adds: without the setting, every request
seems to come from the proxy. With it, the server takes the address the
outermost proxy saw, and ignores what a client wrote there itself.

### Operating it

- **Upgrades.** Pull the new image (or change the tag) and start the
  container again: the server applies the database migrations as it starts.
  Running sessions carry on, and their hosts reconnect. Hosts and agents
  tolerate a control plane one version ahead.
- **Backups.** Everything is in the data directory. Stop the server and
  copy it, or snapshot its volume: `docker run --rm -v fctl-data:/data -v
  "$PWD":/backup busybox tar czf /backup/fctl-data.tgz /data`. Unless you
  set `FCTL_MASTER_KEY`, the copy holds the master key too: keep it as
  safe as the key.
- **Logs.** `docker logs fctl`, or `kubectl -n fctl logs
  statefulset/fctl-control`: what the control plane logs, and its errors.
  `fctl host logs` shows a host's.
- **Housekeeping.** The server runs the hourly job of
  [Deletion](architecture.md#data) at 17 minutes past each hour, while it
  runs.
- **What it takes.** About 500 MB of memory and little CPU at rest. The
  data grows with sessions' transcripts and attached files.
- **Removing it.** `fctl host uninstall` on each host first, while the
  control plane can still stop their sessions (`--force` stops the ones
  that run), then remove the container and its volume.

## On Cloudflare

An instance on Cloudflare is one Worker (the API, the web app and its
Durable Objects) and a D1 database, on your Cloudflare account and domain.

### What you need

- A Cloudflare account with a domain (zone) on it. The **Workers Paid** plan
  ($5 a month) is recommended for its higher limits; a small team stays well
  inside what it includes.
- Rust (via [rustup](https://rustup.rs); the repository pins its toolchain)
  and Node.js 22 or newer.
- Wrangler signed in to that account: run `npx wrangler login` in the
  repository, or export `CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID`.
  An API token needs **Workers Scripts: Edit**, **D1: Edit**,
  **Workers KV Storage: Edit**, **Account Settings: Read**, and, on the zone,
  **Workers Routes: Edit** and **DNS: Edit** (for the custom domain).

### 1. Describe the instance

```sh
git clone https://github.com/hyperstructure/fctl && cd fctl
cp deploy/instances/example.toml deploy/instances/acme.toml
```

Edit `deploy/instances/acme.toml`:

```toml
worker = "fctl"                         # the Worker's name in your account
instance_name = "Acme factory"          # shown in the app and in emails
public_url = "https://fctl.acme.dev"    # its host becomes the Worker's custom domain

[d1]
database_name = "fctl"
database_id = ""                        # filled in by --create
location = "weur"                       # optional primary location
```

Nothing in this file is secret; commit it to your fork if you like.

### 2. Deploy

```sh
cargo xtask deploy acme --create
```

This creates the D1 database (recording its id in the instance file), builds
the web app and the Worker (Rust compiled to WebAssembly), renders
`crates/fctl-control/wrangler.deploy.toml`, applies the database migrations,
and deploys to your domain. Run it again, without `--create`, for every
upgrade.

### 3. Set the secrets

```sh
cargo xtask secrets acme
```

This sets, once:

- `FCTL_MASTER_KEY` — 32 random bytes that encrypt stored credentials and
  sign state. Keep it: replacing it makes every stored Claude credential and
  project secret unreadable.
- `FCTL_SETUP_TOKEN` — a one-time token. The command prints the setup link,
  `https://fctl.acme.dev/setup#token=…`. Open it to create the account of
  the primary instance owner; after that, the link does nothing.
  `--new-setup-token` prints a fresh one.

Other secrets go in with `--set`, and the GitHub App's key with
`--github-key` (below).

### Operating it

- **Upgrades.** `git pull && cargo xtask deploy acme`; migrations apply first.
  Hosts and agents tolerate a control plane one version ahead.
- **Backups.** D1 keeps 30 days of point-in-time history (Time Travel):
  `npx wrangler d1 time-travel restore`. For a copy, `npx wrangler d1 export`.
  Session event logs live in Durable Object storage.
- **Logs.** `npx wrangler tail fctl` streams the Worker's logs; observability
  is enabled in the rendered config. `fctl host logs` shows a host's.
- **Costs.** The control plane is request- and storage-priced on Cloudflare;
  the work happens on your hardware and your Claude subscriptions. fctl never
  makes Anthropic API calls.
- **Removing it.** `fctl host uninstall` on each host first, while the
  control plane can still stop their sessions (`--force` stops the ones
  that run), then `npx wrangler delete fctl` and
  `npx wrangler d1 delete fctl`.

## The first account and your team

Open the setup link to create the account of the primary instance owner,
then sign in and create your first workspace. Then invite your team under
**Admin → People** (each address on its own, and to your workspaces at the
same time), or allow them under **Admin → Access**: exact emails (which
also creates invite links) or whole domains (`*@acme.dev`), whose people
then sign up with Google and join the workspaces that invite them. To require
two-factor sign-in of everyone, turn it on under **Admin → Settings →
Sign-in** (or `fctl admin set --require-two-factor on`); it seals each
authenticator app's secret with `FCTL_MASTER_KEY`. Passkeys belong to the
host name in `FCTL_PUBLIC_URL`: moving the instance to another host name
leaves them behind, and people add new ones.

## Connect GitHub (optional, recommended)

With a GitHub App, projects link repositories, agents get short-lived tokens
for their project's repositories only, and fctl can create new repositories.
An instance has one app; each workspace works in the accounts it is
installed on that are connected to the workspace.

1. Create an app under your organization: **Settings → Developer settings →
   GitHub Apps → New GitHub App**. Turn the webhook off. Set its **Setup
   URL** to `https://fctl.acme.dev/api/v1/github/installed` (your
   `FCTL_PUBLIC_URL` followed by `/api/v1/github/installed`) and tick
   **Redirect on update**, so that people who install it from a workspace
   come back to it. Repository permissions:

   | Permission | Access | Why |
   |---|---|---|
   | Contents | Read and write | clone, commit, push |
   | Pull requests | Read and write | open and update pull requests |
   | Issues | Read and write | agents can read and comment on issues |
   | Workflows | Read and write | agents may edit `.github/workflows` |
   | Checks, Actions, Commit statuses | Read | agents can see CI results |
   | Administration | Read and write | only for **New repository** |
   | Metadata | Read | required |

2. Install it on the organization (all repositories, or the ones agents may
   touch, including those they only read, such as private dependencies).
   To let workspaces use other accounts (someone's own, a friend's
   organization), make the app public (**Advanced → Make public**): only
   then can others install it. Installing it lets the instance mint tokens
   for that account, but no workspace uses it until it is connected to the
   workspace.
3. Generate a private key, and tell the control plane about the app.

   On your own machine, set `GITHUB_APP_ID`, `GITHUB_APP_SLUG`,
   `GITHUB_CLIENT_ID`, `GITHUB_DEFAULT_OWNER` (the organization **New
   repository** creates repositories in) and `GITHUB_APP_PRIVATE_KEY`, the
   key as GitHub gives it (with Kubernetes, in the `fctl` Secret:
   `--from-file=GITHUB_APP_PRIVATE_KEY=acme-fctl.private-key.pem`), and
   start the server again.

   On Cloudflare, fill in the instance file:

   ```toml
   [github]
   app_id = "123456"
   app_slug = "acme-fctl"
   client_id = "Iv23li…"
   default_owner = "acme"     # where "New repository" creates repositories
   ```

   Then store the key and redeploy:

   ```sh
   cargo xtask secrets acme --github-key ~/Downloads/acme-fctl.private-key.pem
   cargo xtask deploy acme
   ```

The key is stored as PKCS#8, which WebCrypto imports (`cargo xtask secrets`
and the server convert it); it never reaches a host or a sandbox.

### Which workspace uses which account

A workspace's admins install the app from the workspace's **GitHub** tab,
or name an account it is installed on, and the workspace works there once
an instance admin approves it under **Admin → GitHub** (`fctl admin github
approve <account> <workspace>`). GitHub sends the installation's id to the
setup URL, where anyone could send another account's, so connections take
an instance admin; an instance admin's own connect at once. A connection
can be narrowed to some of the account's repositories (`--repo`). New
repositories go into `default_owner` when the workspace works in all of it,
otherwise into an organization the workspace works in all of; the app can
create repositories in organizations, not in people's own accounts.

### What sessions get

A project has a primary repository and may list more, each read only or
read and write, or let agents read every repository of the account that the
app can reach (`owner/*`, within what its workspace may use); they all
belong to one account, whose installation mints the tokens. Each session gets up to two, valid for an
hour and refreshed through its host:

| Token | Repositories | Permissions |
|---|---|---|
| Write | the primary repository and those listed as read and write | contents, pull requests, issues and workflows: write; checks, actions, commit statuses: read |
| Read only | those listed as read only, or the account's (`owner/*`) | contents, pull requests, issues, checks, actions, commit statuses: read |

Agents may write only to repositories that are named: reading a whole
account is as far as an owner-wide grant goes. A listed repository the
installation no longer reaches (removed from it, renamed or deleted) is left
out of the tokens, and the session says so; a session whose primary
repository is out of reach does not start. People manage the list under the
project's **Repositories** tab or with `fctl projects repos`.

## Google sign-in (optional)

Create an OAuth client of type **Web application** in the Google Cloud
console, with the authorized redirect URI
`https://fctl.acme.dev/api/v1/auth/google/callback` (your `FCTL_PUBLIC_URL`
followed by `/api/v1/auth/google/callback`; Google takes http ones only for
`localhost`).

On your own machine, set `GOOGLE_CLIENT_ID` and `GOOGLE_CLIENT_SECRET` and
start the server again. On Cloudflare:

```toml
[google]
client_id = "1234-abc.apps.googleusercontent.com"
```

```sh
echo "$GOOGLE_CLIENT_SECRET" | cargo xtask secrets acme --set GOOGLE_CLIENT_SECRET
cargo xtask deploy acme
```

People can then link Google under **Settings**, and allowlisted people can
sign up with it.

## Email (optional)

Without email, admins copy invite and password-reset links from the app. To
send them, use [Resend](https://resend.com) with a verified domain.

On your own machine, set `RESEND_API_KEY` and `FCTL_EMAIL_FROM`
(`Acme factory <fctl@acme.dev>`) and start the server again. On Cloudflare:

```toml
[email]
from = "Acme factory <fctl@acme.dev>"
```

```sh
echo "$RESEND_API_KEY" | cargo xtask secrets acme --set RESEND_API_KEY
cargo xtask deploy acme
```

## Bring hosts online

On each machine that runs agents (the one that runs the control plane can
be one of them), follow
[Getting started → Bring a machine online](getting-started.md#5-bring-a-machine-online):

```sh
curl -fsSL https://fctl.sh/install | sh
fctl login fctl.acme.dev            # or with its scheme: fctl login http://192.168.1.20:8787
fctl host install
```

Hosts download their release from `https://fctl.sh/releases` by default; set
`download_base` in `host.toml` to mirror them elsewhere (`fctl host image
build` installs fctl in the default image from there too). They pull the
sandbox image from `ghcr.io/hyperstructure/sandbox`; set `image` to use a
copy in your own registry ([Sandbox images](sandbox-images.md)). A host runs
any image a project chooses until its owner lists the ones it allows
(`allowed_images`), and builds images from no repository until they list
some (`allowed_builds`): set both on machines that run other people's
projects ([Which images a host runs](sandbox-images.md#which-images-a-host-runs)).

## Notifications and published screenshots

- **Notifications** need no setup and no push provider account: the control
  plane makes its Web Push (VAPID) key the first time someone opens
  Settings → Notifications, and keeps it in D1, sealed with the master key.
  Browsers allow notifications only from `https://` origins (and
  `http://localhost`).
- **Published screenshots** stay off until an admin allows them (Admin →
  Settings, `fctl admin set --publish-screenshots on`) and a project turns
  them on. Its agents then publish screenshots for pull requests at links
  on your instance's domain, which anyone who has one can open until they
  expire (30 days unless the project chooses 24 hours to 60 days). If your
  instance has a privacy policy, say so there. GitHub shows them in pull
  requests only when its image proxy can fetch them, so only from an
  instance it can reach.
