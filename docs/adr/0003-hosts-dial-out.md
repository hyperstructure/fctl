# 0003: Hosts dial out, and every session gets its own VM

2026-10-17 · accepted

## Context

Agents run on people's own machines (a spare Mac, a Linux box) behind home
or office networks, with permission prompts off.

## Decision

`fctl host` opens outbound WebSockets to the control plane (one control
connection, one uplink per session) and never listens on a port. Each
session runs in its own sandbox: a lightweight VM with Apple's container
runtime on Apple silicon, a Lima VM elsewhere on macOS, or a Docker
container on Linux. The `fctl-agent` supervisor inside it runs detached, so
restarting or upgrading the host does not stop sessions.

## Consequences

- No tunnels, port forwarding or public addresses are needed.
- An agent can do anything inside its VM and nothing outside it except
  through its GitHub token (one repository), its Claude access token and the
  project's secrets.
- Provisioning a VM per session costs seconds; images are built once per
  host (`fctl host image build`).
