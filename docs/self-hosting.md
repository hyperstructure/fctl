# Self-hosting

An fctl instance is one Cloudflare Worker (the API, the web app and two
Durable Object classes) plus a D1 database, on your own Cloudflare account and
domain. Hosts run on your machines and connect to it. Nothing in the code is
tied to fctl.sh: everything instance-specific is configuration.

## What you need

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

## 1. Describe the instance

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

## 2. Deploy

```sh
cargo xtask deploy acme --create
```

This creates the D1 database (recording its id in the instance file), builds
the web app and the Worker (Rust compiled to WebAssembly), renders
`crates/fctl-control/wrangler.deploy.toml`, applies the database migrations,
and deploys to your domain. Run it again, without `--create`, for every
upgrade.

## 3. Set the secrets and create the owner

```sh
cargo xtask secrets acme
```

This sets, once:

- `FCTL_MASTER_KEY` — 32 random bytes that encrypt stored credentials and
  sign state. Keep it: replacing it makes every stored Claude credential and
  project secret unreadable.
- `FCTL_SETUP_TOKEN` — a one-time token. The command prints the setup link,
  `https://fctl.acme.dev/setup#token=…`. Open it to create the owner account;
  after that, the link does nothing. `--new-setup-token` prints a fresh one.

Sign in, then allow your team under **Admin → Access**: exact emails (which
also creates invite links) or whole domains (`*@acme.dev`).

## 4. Connect GitHub (optional, recommended)

With a GitHub App, projects link repositories, agents get short-lived tokens
for their project's repositories only, and fctl can create new repositories.

1. Create an app under your organization: **Settings → Developer settings →
   GitHub Apps → New GitHub App**. Turn the webhook off. Repository
   permissions:

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
3. Generate a private key, and fill in the instance file:

   ```toml
   [github]
   app_id = "123456"
   app_slug = "acme-fctl"
   client_id = "Iv23li…"
   default_owner = "acme"     # where "New repository" creates repositories
   ```

4. Store the key and redeploy:

   ```sh
   cargo xtask secrets acme --github-key ~/Downloads/acme-fctl.private-key.pem
   cargo xtask deploy acme
   ```

The key is converted to PKCS#8 and stored as the `GITHUB_APP_PRIVATE_KEY`
secret; it never reaches a host or a sandbox.

### What sessions get

A project has a primary repository and may list more, each read only or
read and write, or let agents read every repository of the account that the
app can reach (`owner/*`); they all belong to one account, whose
installation mints the tokens. Each session gets up to two, valid for an
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

## 5. Google sign-in (optional)

Create an OAuth client of type **Web application** in the Google Cloud
console, with the authorized redirect URI
`https://fctl.acme.dev/api/v1/auth/google/callback`. Then:

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

## 6. Email (optional)

Without email, admins copy invite and password-reset links from the app. To
send them, use [Resend](https://resend.com) with a verified domain:

```toml
[email]
from = "Acme factory <fctl@acme.dev>"
```

```sh
echo "$RESEND_API_KEY" | cargo xtask secrets acme --set RESEND_API_KEY
cargo xtask deploy acme
```

## 7. Bring hosts online

On each machine that runs agents, follow
[Getting started → Bring a machine online](getting-started.md#4-bring-a-machine-online):

```sh
curl -fsSL https://fctl.sh/install | sh
fctl login fctl.acme.dev
fctl host install
```

Hosts download their release from `https://fctl.sh/releases` by default; set
`download_base` in `host.toml` to mirror them elsewhere (`fctl host image
build` installs fctl in the default image from there too). They pull the
sandbox image from `ghcr.io/hyperstructure/sandbox`; set `image` to use a
copy in your own registry ([Sandbox images](sandbox-images.md)).

## Operating it

- **Upgrades.** `git pull && cargo xtask deploy acme`; migrations apply first.
  Hosts and agents tolerate a control plane one version ahead. Upgrade hosts
  by running the installer again and `fctl host restart` (running sessions
  continue): new sessions start from the sandbox image of the new version,
  which the host pulls when it starts.
- **Backups.** D1 keeps 30 days of point-in-time history (Time Travel):
  `npx wrangler d1 time-travel restore`. For a copy, `npx wrangler d1 export`.
  Session event logs live in Durable Object storage.
- **Logs.** `npx wrangler tail fctl` streams the Worker's logs; observability
  is enabled in the rendered config. `fctl host logs` shows a host's.
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
  instance has a privacy policy, say so there.
- **Costs.** The control plane is request- and storage-priced on Cloudflare;
  the work happens on your hardware and your Claude subscriptions. fctl never
  makes Anthropic API calls.
- **Removing it.** `npx wrangler delete fctl` and
  `npx wrangler d1 delete fctl`; `fctl host uninstall` on each host.
