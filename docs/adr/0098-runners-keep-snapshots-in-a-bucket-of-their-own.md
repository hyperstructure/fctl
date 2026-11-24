# 0098: Runners keep snapshots in a bucket of their own

2026-11-23 · accepted

## Context

Sessions keep their conversation and unpushed work in snapshots that the
control plane stores, encrypted
([ADR 0096](0096-sessions-keep-their-conversation-in-encrypted-snapshots.md)).
Some organizations want their code and their agents' conversations never
to leave their own cloud, even encrypted, and some sessions' snapshots
outgrow what an instance keeps (200 MiB by default). Their runners run in
that cloud already, with its credentials (#615).

## Decision

- **A host can name a bucket of its own** in `host.toml` (`[snapshots]
  url = "s3://bucket/prefix"`): its sessions' snapshots go there, sealed
  as before, and the control plane keeps only where each is, its size and
  its checksum. Hosts, not workspaces, choose: the bucket is where the
  machines are, with their credentials, and a workspace's runners may be
  in several clouds.
- **S3's API, signed with SigV4 in fctl** (`fctl_domain::sigv4`), rather
  than AWS's SDK: one API that R2, MinIO and Google Cloud Storage also
  take, a small signer tested against AWS's published examples, and
  credentials found where AWS's own tools look (keys in host.toml, the
  `AWS_*` variables, a web identity, a container's credentials, the
  instance's role). Larger snapshots go in parts, so that the host's
  memory holds 16 MiB at a time.
- **The host's limit, not the instance's**, holds in its bucket
  (`max_mb`, 2048 MiB by default): the instance's protects its own
  storage.
- **A resume goes to a runner with the same bucket.** Other runners can't
  read it, so FleetDO prefers those, and waits for one that is busy as
  long as a resume waits for its sandbox's runner (ten minutes); where
  none can take the session, it starts from its branch, as without
  snapshots.
- **Deleting is the host's and the bucket's.** A host that saves a
  session's snapshot deletes the one before, which the commit's answer
  names, when it is in its own bucket. A location names only the
  session's own key, so that no runner points one session at another's
  object. The control plane forgets one past its workspace's days but
  can't delete it: the bucket's lifecycle rules do, which the docs set
  up.
- **A URL the host can't read stops it**, so that snapshots meant for the
  bucket never go to the control plane by mistake. A bucket that fails
  is said on the Runners tab and in `fctl runners`, and a snapshot it
  didn't take is lost, not sent elsewhere.

## Consequences

- An organization's runners keep its sessions' code and conversations in
  its own cloud; app.fctl.sh keeps only where they are.
- A bucket's lifecycle rules, not the workspace's days, decide when its
  snapshots go: the two should agree.
- A session that moves to a runner without that bucket starts from its
  branch.
- Older hosts ignore `[snapshots]`, which they don't know, and keep
  sending snapshots to the control plane.
