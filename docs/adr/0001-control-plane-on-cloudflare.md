# 0001: The control plane is a Cloudflare Worker with Durable Objects, written in Rust

2026-10-17 · accepted

## Context

The control plane holds accounts, projects and sessions, fans session
events out to everyone watching, and hands work to hosts. It must be up all
the time, cost little at small scale, and be easy for others to run.

## Decision

One Worker (`crates/fctl-control`, Rust compiled to wasm with workers-rs)
serves the API and the web app's static assets. Relational state lives in
D1. Each session is a Durable Object (`SessionObject`) that owns its event
log, its viewers' WebSockets and its message outbox; one `FleetObject` owns
the runners' control connections and scheduling. WebSockets use the
hibernation API, so idle connections cost nothing.

## Consequences

- A session's state has one writer, so ordering and fan-out need no locks.
- Deploying an instance is one command (`cargo xtask deploy`) on any
  Cloudflare account; the free tier of most bindings covers a small team.
- The control plane code shares its types with the host and the CLI
  (`fctl-proto`), and a panic or failing SQL statement inside a Durable
  Object can poison it, so that crate denies `unwrap`/`panic` and its
  migrations are idempotent.
