# 0073: Changing review agents runs them on the subscription of whoever changes them

2026-11-05 · accepted

## Context

Review agents run on the Claude subscription of the project owner who
turned them on, their operator
([0040](0040-review-agents-review-pull-requests-on-an-owners-subscription.md)).
Another owner took them over only by asking to (`run_as_me`, **Run them on
mine**). Any owner could change how they review without that: their
model and effort level, the instructions they get, the repositories they
skip, whether drafts get reviews, whether they sum pull requests up, and
the apps whose pull requests start reviews. So one owner, or a workspace
admin, could change what runs on another person's subscription, and how
much of it, without that person agreeing. They might not notice until
their plan ran out.

Schedules already work the other way
([0044](0044-schedules-start-sessions-on-their-owners-subscription.md)): a
change by anyone but their owner runs them on the subscription of whoever
made it.

## Decision

- **A change by another owner takes them over.** When an owner who isn't
  the operator saves a change to how review agents review while they are
  on, they run on that owner's subscription from then on, as when they
  take them over. That owner needs a Claude connection for the workspace.
  Only a real change counts: saving the settings as they are changes
  nothing.
- **Turning them off moves nothing.** It puts no work on anyone's
  subscription. While they are off nothing runs, and whoever turns them
  on again takes them over, as before.
- **The operator's own changes keep them where they are**, and the
  operator may still always turn them off.

## Consequences

- Nobody changes what runs on someone else's subscription: the person
  who decides how review agents review pays for those reviews.
- An owner who adjusts another's review agents takes on their reviews.
  The settings form says so before they save, the toast after, and the
  audit log records whose subscription they were taken from
  (`taken_over_from`).
- An owner without a Claude connection can no longer change review
  agents that run on someone else's subscription. They can still turn
  them off.
