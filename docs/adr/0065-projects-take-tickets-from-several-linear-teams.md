# 0065: Projects take tickets from several Linear teams

2026-10-29 · accepted

## Context

[0052](0052-projects-choose-a-ticket-tracker-and-linear-delegates-tickets-to-fctl.md)
had a project take the tickets of one Linear team, optionally narrowed to
one of its Linear projects: delegations and mentions of the team's issues
started its sessions, and its agents reached that team's tickets alone.
Work that one repository holds is often tracked by several teams, a
product team and a platform team say, whose tickets then had no project
to start sessions in, or needed a project per team on the same
repository.

The person who started session #485 asked for a project to take tickets
from several Linear teams, with a web app that makes choosing them easy.

## Decision

- **A project takes the tickets of one team or several**, at most 20, in
  the Linear workspace it chose, each team once, and each of all its
  issues or of one of its open Linear projects. Delegations and mentions
  of any of them start its sessions; one that takes only the issue's
  Linear project comes before projects that take the whole team, as
  before.
- **The first team stays where it was.** `LinearTickets` keeps its first
  team in the fields that held the one (`team`, `team_key`, `team_name`,
  `project`, `project_name`), and the others in a list after it
  (`more_teams`, `LinearTeamChoice`), so that control planes, hosts and
  clients from before it read a project of several as one of its first
  team. An update that leaves the list out keeps the teams stored, less
  the one it makes first, while the Linear workspace stays the same: an
  older CLI or a tab opened before a deploy can't drop them. `[]` says
  there are none.
- **Agents file new tickets in the first team** unless they name another
  (`fctl ticket create --in DES`, the `place` that names a repository for
  GitHub Issues of several), or file a sub-ticket, which goes to its
  parent's team. A ticket's state and labels come from its own team's,
  whose workflow may differ from the others': `fctl ticket states` lists
  those of every team, each name once, and a state a team lacks is
  refused with that team's states.
- **The switches that move tickets stay one per project**, each move
  going to the states of the ticket's own team; the Tickets tab names the
  states each team has for a move ("In Review or Code review").
- **Saving checks with Linear only what changed**: a team checked before
  with the same Linear project keeps what Linear said of it.
- **The Tickets tab lists the teams**, each with the issues of it the
  project takes: a team added since the project was saved shows as new,
  one taken off stays struck through until it is saved, to undo, and once
  there are several, a line under the list chooses where agents file new
  tickets, which puts that team first. `fctl projects set --linear-team
  ENG,DES` sets them, and `--linear-project` takes `DES:Website` to name
  a team's Linear project.

## Alternatives

- **A list of every team, the first included, replacing `team`.**
  Simpler to read, but `team` is required where older versions read it:
  they would fail to read the project's settings at all, or drop the
  first team.
- **The teams in a list, with `team` kept as a copy of the first.** Two
  places to keep alike, and an older client that changes `team` leaves
  them apart.
- **A project per team.** What people did; each team's tickets then need
  a project of their own on the same repository, with its own settings,
  members and memory.
- **Moves switched per team.** More to configure for every team; one
  team whose Linear GitHub integration moves tickets differently can be
  given a project of its own.
- **Several Linear workspaces in a project.** Each has its own
  installation and app user; none of the people who asked needed it.

## Consequences

- A project of one team reads, stores and behaves as before. A control
  plane rolled back to before this takes a project of several as one of
  its first team, and drops the others the next time the project is
  saved there.
- Listing tickets asks Linear for those of every team in one query (`or`
  filters), and `fctl ticket states` and `labels` for every team's in
  one query, under 3,000 points each; filing and changing a ticket asks
  about its own team alone.
- fctl stores more Linear team ids and names in a project's settings, as
  it stored one.
