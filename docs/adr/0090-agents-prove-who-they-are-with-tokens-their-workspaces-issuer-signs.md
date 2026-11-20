# 0090: Agents prove who they are with tokens their workspace's issuer signs

2026-11-20 · accepted

## Context

Organizations want their agents to reach their own systems: an internal
API, a deploy service, secrets in Vault, an AWS account. Until now an
agent could only be handed a long-lived secret, in a project's secrets,
which anything in the sandbox reads and nothing ties to a session, a
person or a runner, and which someone has to rotate.

CI systems solved the same problem with OpenID Connect: GitHub Actions,
GitLab and others sign a short-lived token for each job, which clouds
trust through a trust policy and trade for credentials of their own.
Cursor's cloud agents mint such tokens too
([Cursor's identity docs](https://cursor.com/docs/cloud-agent/identity)).
Session #1511 was asked for the same: agents that mint short-lived tokens
proving that they come from fctl, for which person, project and
workspace, which anyone can check against public keys (app.fctl.sh's, or
a self-hosted instance's own), with the claims a verifier needs, and
organizations that let them assume roles in AWS and the like.

## Decision

- **Each workspace is an issuer of its own**,
  `{FCTL_PUBLIC_URL}/oidc/{wsp_…}`, with a discovery document and keys at
  `{issuer}/.well-known/…` that anyone may read. A cloud set up to trust
  one workspace's agents then trusts no other workspace's, on app.fctl.sh
  or anywhere, even when its conditions leave out the subject: the
  mistake that has let any GitHub repository's jobs into roles that
  trusted GitHub's one issuer. The workspace's id, not its slug, names
  it, as slugs change.
- **The instance's keys sign for every issuer.** RS256, which every cloud
  and library takes; 2048-bit RSA made with WebCrypto as one is first
  needed, sealed with the master key in D1, so that no instance needs a
  key of its own made and deployed. A new one comes every 30 days, is
  published two days before it signs and stays two days after; the
  signature covers `iss`, so no token moves between workspaces.
- **Agents mint them in their sandbox, through their host**
  (`fctl id-token`, `AgentCall::IdToken`), as they reach the memory: the
  sandbox holds no fctl credential, and the control plane signs only for
  a session whose sandbox runs on the runner that asks. It fills the
  claims from what it knows of the session; the agent chooses only the
  audience (any: verifiers check theirs), a nonce, which facts make up the
  subject, and AWS session tags. A token lasts 10 minutes.
- **The claims**: the registered ones; the workspace, project and session,
  by id and by name; the person it runs for, by id and the address they
  are a member of the workspace as; the runner, and whether it runs only
  its owner's sessions; the repository and branches; its GitHub access;
  what started it, in the words webhooks use, with the schedule, ticket,
  autopilot or service account. `sub` is `project:prj_…:user:usr_…`, so
  that AWS's `StringLike` picks a project's or a person's agents;
  `--subject` builds another of the same facts for clouds that match
  exactly, as Azure does (`project:prj_…`). Ids rather than slugs, as a
  deleted project's slug can be another's later.
- **Projects reach clouds without the agent minting anything.** Their
  maintainers name an AWS role, a Google Cloud workload identity provider
  or an Azure application (`ProjectSettings::cloud_access`). fctl-agent
  keeps a token for each in a file, minted again every five minutes, and
  sets the variables the cloud's own CLI and SDKs read
  (`AWS_WEB_IDENTITY_TOKEN_FILE`, `GOOGLE_APPLICATION_CREDENTIALS`,
  `AZURE_FEDERATED_TOKEN_FILE`…), which trade it for the cloud's
  credentials and do again as those run out. fctl never calls a cloud
  and holds none of its credentials. `GET /api/v1/projects/{id}/identity`
  (Settings → Identity, `fctl projects identity`) gives the provider, the
  trust policy and the commands each cloud needs, filled in.
- **On by default, off for review agents.** A token reaches nothing until
  someone sets a service or a cloud up to trust the workspace's issuer,
  and they choose what it lets in. A project's maintainers turn them off
  (`ProjectSettings::id_tokens`), which also ends its clouds. Review
  agents never mint them, as they keep no memory: the pull requests they
  review could steer what runs in their sandbox.

## Alternatives

- **One issuer for the instance.** Simpler URLs (`app.fctl.sh`), but on
  a shared instance a role whose trust policy forgets the subject trusts
  every customer's agents.
- **A key the deployment provides** (a secret, as the GitHub App's).
  Every instance would need one made, deployed and rotated by hand, and
  the standalone server another setting; the control plane already makes
  and seals its Web Push key the same way.
- **ES256.** Smaller and faster, but not every verifier takes it: RS256
  is what GitHub, GitLab and Cursor sign with.
- **Tokens from the API, for people or access tokens.** A person outside
  the sandbox could then pose as an agent; the point is to prove a
  request comes from a running session.
- **A `credential_process` that calls AWS itself.** fctl would talk to
  each cloud and parse its answers; the token files are what EKS, GitHub
  and the clouds' own SDKs already know.
- **Allowed audiences per workspace.** Verifiers check the audience
  anyway; an allowlist would only make agents ask an admin before they
  call a new service.

## Consequences

- Whatever runs in a session's sandbox can mint its tokens, and so can
  the owner of the runner it runs on: a token proves where a request
  comes from, not what the agent meant. The docs say so, and the claims
  (`github_access`, `started_by`, `runner_personal`) let verifiers leave
  out agents that read outsiders' text or run on members' machines.
- Clouds fetch the issuer's documents themselves, so a self-hosted
  instance's agents reach them only from an `https://` address on the
  internet; `/oidc/*` goes to the Worker ahead of the app's files
  (`run_worker_first`).
- The sandbox's `fctl` needs `fctl id-token`, from the release that brings
  it, and the runner a host of that release; older ones refuse the call,
  and their sandboxes get no cloud variables.
- Keys rotate on their own, and verifiers that cache keys for less than
  two days always have the one they meet.
