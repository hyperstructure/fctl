# 0060: Taken and reserved slugs get a random ending

2026-10-28 · accepted

## Context

A workspace's or project's slug is its address, `/w/<slug>` or
`/p/<slug>`, unique on the instance among workspaces and among projects.
A new one took the slug its name made, and when another had it, a number
after it: the second Acme was `acme-2`, the third `acme-3`. fctl kept the
slugs its own pages use for itself (`new`, `admin`, `settings`…:
`fctl_domain::validate::RESERVED_SLUGS`), and a name that made one of
those got a word more (`new-workspace`) or, for a project, `project-` and
part of a ULID. Instance admins had no way to hold an address for later,
such as `home` or `github`, for something the instance may serve there
one day.

Session #391 was asked to let instance admins and the primary instance
owner reserve slugs, starting from none, and to give a new workspace or
project whose name makes a reserved or taken slug that slug with six
random hex digits after it, `example-829d34`, instead of `-2` and `-3`.

## Decision

- **Instance admins reserve slugs.** `reserved_slugs` holds them, with who
  reserved each and when. Admins and the primary instance owner list, add
  and remove them (`/api/v1/admin/reserved-slugs`; `fctl admin reserved`,
  `reserve` and `unreserve`; Admin → Settings → Reserved addresses), and
  the instance's audit log records each change (`reserved_slug.added`,
  `reserved_slug.removed`). A new instance has none. A reserved slug holds
  the address for workspaces and projects both. fctl's own aren't in the
  list: reserving one answers that fctl reserves it itself, and nobody
  unreserves it.
- **A taken or reserved slug gets a random ending.** A new workspace or
  project takes the slug its name makes while no other has it and nobody
  reserves it. Otherwise it takes that slug, cut to 41 characters, with a
  dash and six random hex digits, checked as the slug itself was
  (`crate::slugs::free`, `fctl_domain::slugs::with_ending`). fctl's own
  reserved slugs follow the same rule: `New` makes `new-3bf1bb`, no longer
  `new-workspace`.
- **Nobody chooses a reserved slug.** A `slug` asked for when creating a
  workspace or project, or as a workspace's new address, is refused when
  it is reserved, as when it is taken ("This name is reserved"). A
  workspace that sends its own address back is never refused, reserved
  since or not.
- **What has one keeps it.** Reserving a slug that a workspace or project
  has already changes nothing for it; only new ones are held back.
- **The New workspace page shows it where it can.** Its preview of the
  address shows where the random ending goes (`acme-xxxxxx`) and why, for
  fctl's own slugs and those of the person's own workspaces. The server
  settles the rest, the addresses of other people's workspaces and the
  slugs admins reserve, which the page doesn't know.

## Alternatives

- **A list in the instance's settings** (`InstanceSettings`), saved with
  them: every request reads those settings, two admins changing the list
  at once could each undo the other's change, and it would record nobody
  as having reserved a slug.
- **Separate lists for workspaces and projects.** Nobody asked to hold an
  address for one but not the other, and the examples asked about (`me`,
  `settings`, `login`, `users`, `github`) are names the app itself might
  want for either.
- **Random endings for chosen slugs too**, rather than refusing them:
  someone who typed an address would get another one without being told.
- **Letting admins choose a reserved slug.** They unreserve it first,
  which keeps one rule for everyone, and the audit log shows it.

## Consequences

- The second Acme's address is `acme-229b4a`, not `acme-2`: people copy
  addresses rather than guess them, and an ending says nothing about how
  many workspaces or projects of that name came before.
- Workspaces and projects made before keep their numbered slugs.
- A workspace whose address is reserved after it took it keeps it, but
  once it moves to another address, it can't go back to that one.
- Anyone can learn whether a slug is taken or reserved by asking for it,
  as they could learn before that one was taken. The list, and who
  reserved each slug, only instance admins see.
