# 0026: The control plane also runs without Cloudflare, on workerd

2026-10-20 · accepted

## Context

The control plane is one Cloudflare Worker with Durable Objects and D1
([ADR 0001](0001-control-plane-on-cloudflare.md)), and deploying an
instance took a Cloudflare account with a domain on it, Wrangler signed in,
and an API token with five permissions. That is what app.fctl.sh wants:
Cloudflare keeps it up, backs D1 up and serves it from near everyone. For
someone trying fctl out, on a spare machine, a homelab or a Kubernetes
cluster they already run, a new Cloudflare account is a lot to ask before
the first session.

The Worker is 20,000 lines of Rust on workers-rs, which binds to the
Workers runtime's JavaScript APIs: Request and Response, D1, Durable
Objects with their SQLite storage, alarms and hibernating WebSockets,
WebCrypto. Two ways to run it elsewhere:

- **A native server**: the same crate compiled for the host, with a
  second implementation of every API it uses (a SQLite-backed D1, Durable
  Objects as in-process actors with alarms and WebSockets, WebCrypto's
  ECDSA, ECDH and RSA). A single small binary, but two runtimes behind one
  codebase: every change to the control plane would have to keep both
  right, and the second would drift from the one that runs app.fctl.sh in
  ways only production would show.
- **The Workers runtime itself.** workerd, the runtime the Workers
  platform is built on, is open source and runs this Worker unchanged.
  Miniflare, Cloudflare's library around it, adds D1, the rate limiter and
  static assets with the `_headers` file, keeps D1 and every Durable Object
  in SQLite files in a directory, and persists alarms across restarts.
  `wrangler dev` is Miniflare, so `cargo xtask dev` and CI's smoke test
  have run the control plane on it all along.

## Decision

- **workerd, through Miniflare.** `deploy/standalone/server.mjs` runs the
  Worker and the web app as built for Cloudflare, with the bindings of
  `crates/fctl-control/wrangler.toml`, which wrangler turns into Miniflare's
  options, and its state in `FCTL_DATA_DIR`. No code in the Worker knows
  where it runs. Its configuration is the same variables and secrets,
  from the environment.
- **The server does what Cloudflare does around the Worker.** It applies
  the D1 migrations before it starts (`wrangler d1 migrations apply
  --local`, so the `d1_migrations` table records them as on Cloudflare),
  and runs the hourly cron trigger, which Miniflare doesn't. Its front
  proxy sets `CF-Connecting-IP` from the connection, or from
  `X-Forwarded-For` behind `FCTL_TRUST_PROXY` proxies, as the edge does:
  Miniflare keeps one a client sends, and would let it steer Miniflare
  with `MF-*` headers or reach its pages under `/cdn-cgi/`.
- **One server per data directory.** Durable Objects assume one instance
  each; two servers on the same files would each dispatch the same
  sessions. The server holds a lock file it touches every few seconds, and
  takes over one left untouched for 30 seconds.
- **A container image**, `ghcr.io/hyperstructure/fctl-control:<version>`,
  for amd64 and arm64, published with every release, and examples for
  Docker Compose and Kubernetes (a one-replica StatefulSet). `cargo xtask
  serve` runs the same server from a clone. Without `FCTL_MASTER_KEY`, the
  server makes one in the data directory, and without `FCTL_SETUP_TOKEN` a
  setup link at each start, which it prints until the first account
  exists: one `docker run` makes an instance.
- **Plain http works** at any address, for a machine on a home network:
  where browsers refuse `__Host-` cookies (plain http outside localhost),
  the control plane names them without the prefix and without `Secure`
  (`fctl_domain::cookies`). The server says, as it starts, what that gives
  up: passkeys, notifications and the installable app need https, and
  anyone on the network reads the cookies.
- **Cloudflare stays the way app.fctl.sh runs**, and the one this ADR
  doesn't change.

## Consequences

- An instance takes a machine with Docker, or Node.js and a clone, and no
  account anywhere. The same hosts, CLI and web app work with both.
- The two ways run the same code on the same runtime, and CI runs a
  session through each: `cargo xtask dev --smoke` through `wrangler dev`,
  and the image's job through the container.
- What Cloudflare gives app.fctl.sh, the standalone server leaves to its
  operator: TLS (a reverse proxy), backups (copy the data directory while
  the server is stopped), and availability (one process; no failover).
  Miniflare is made for development: Cloudflare doesn't promise it for
  serving, and a crash of workerd is Miniflare's to restart. The server
  exits when the runtime stops answering, so that Docker or Kubernetes
  starts it again.
- The server needs about 450 MB of memory at rest (Node.js and workerd),
  more than a native binary would.
- Upgrading wrangler upgrades Miniflare and workerd for both `cargo xtask
  dev` and the image; the server takes the Miniflare wrangler depends on.
- The Worker's name and its D1 database's id in `wrangler.toml` name the
  standalone state's files, so they stay as they are.
- `request.cf` is empty outside Cloudflare, so sign-ins record no city or
  country.
