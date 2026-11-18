# 0086: Service accounts start sessions for the people who allow them

2026-11-18 · accepted

## Context

Organizations want places of their own to start agents from: a portal
where their people pick a project and describe a task, a chat bot, a
script that a form calls. Each of those programs has to call fctl for
someone, and the session it starts should be that person's: on their
Claude subscription, within what they may do in the project, followed
by them.

fctl had no credential for that. A person's access token acts as them
alone ([ADR 0037](0037-access-tokens-and-sessions-limit-what-agents-do-on-github.md)),
so a portal would hold a token of each of its users; admin tokens
provision people on self-hosted instances and reach nothing inside
workspaces ([ADR 0085](0085-admin-tokens-provision-people-on-self-hosted-instances.md));
a schedule run by a tool starts on the asker's subscription
([ADR 0044](0044-schedules-start-sessions-on-their-owners-subscription.md)).

A program that starts sessions for others spends their subscriptions
and acts in their name. On app.fctl.sh, where people from many
organizations share an instance, nobody should have that power over
someone who didn't agree to it; an organization that runs its own
instance for its own people may decide for them.

## Decision

- **A workspace's credential.** Its admins make service accounts
  (`fctl_svc_…`), at most ten, each with a name and what it is, which the
  people it asks read. Making one, or a new secret for one, takes
  confirming it's you. One belongs to the workspace, but works only while
  the admin who made its secret is an active admin there, as enrollment
  tokens do: someone who left can't act for people with a secret they
  kept. Another admin makes it a new secret, which stops the old one at
  once.
- **For one person at a time, named in each request**
  (`Fctl-On-Behalf-Of`, by id or address; `fctl --on-behalf-of`). The
  request then acts as them, held to what an access token that runs
  sessions does, in the service account's workspace alone: what a
  session it starts may do comes from their roles, and the session is
  theirs, on their subscription, followed by them. A header rather than
  a field of each body, so that every endpoint, reading ones too, works
  for the person without a change of its own, and the CLI's commands do
  with one flag.
- **For nobody, it reads.** Without a person it reads its workspace's
  projects, sessions and schedules, and runs a schedule for the person
  the schedule runs for, as at its time. It never reaches accounts,
  people, settings, secrets, tokens or administration, nor live
  connections, which check their token while they stay open.
- **People choose, unless the instance decides for them.** An instance
  setting, on by default, has service accounts act only for the people
  who allowed them, each service account on its own: fctl asks each
  member, and each newcomer, whose invitation says so before they join;
  people change their answer whenever they like, and allowing one takes
  confirming it's you. Its admins turn asking off for an instance whose
  people all work for the organization that runs it; the service
  accounts then act for any member, and people's answers count again
  once it's on.
- **It says so everywhere.** Sessions record which service account
  started them; the app, `fctl show`, the audit log (with the person it
  acted for as the actor) and webhooks, which can narrow sessions'
  events to those service accounts started, say it.

## Consequences

- An organization builds its own front for agents on fctl's API, and its
  people find what it started in fctl, as theirs.
- Hosts and agents need no update: a service account's session is a
  session like any other. Older clients ignore `Session.service_account`
  and the new webhook origin reads as `other` to them.
- Whoever holds a service account's secret runs work as the people who
  allowed it, and as anyone in its workspace where the instance doesn't
  ask: as sensitive as the program's own secrets. An admin who leaves
  stops it until another makes it a new secret, which breaks the program
  until it has the new one.
- Asking each person per service account adds a step for every new
  program; it keeps a later, unrelated program from inheriting an answer
  given to another.
- An instance that turns asking off decides for its people, those who
  said no included; members still see which service accounts act for
  them.
