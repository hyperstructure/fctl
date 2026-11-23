# 0097: Admins see facts about members' Claude connections

2026-11-23 · accepted

## Context

Sessions run on each person's own Claude subscription, and organizations
give their people Team or Enterprise seats for it. A workspace's admins
couldn't see who in it had Claude connected, on which plan, whose
connection had stopped working or whose long-lived token ran out next
month: they learned it when someone's sessions failed. fctl couldn't warn
anyone either: it never learned that a long-lived token stopped working,
and the year it recorded for one was a guess it showed nowhere (#627).

A Claude connection is its person's. `AdminClaudeToken`'s documentation
said "their own Claude connections are theirs alone", and on app.fctl.sh a
workspace's admins are often just whoever made it. Organizations running
their own instance want the opposite: one view of every seat. One answer
for both would be wrong for one of them.

## Decision

- **Facts, never the token.** Admins see what a connection is and whether
  it works: its kind and who set it, its plan, when Claude last answered
  it, when its token runs out, whether sessions wait for a usage limit,
  and how often they paused on one. Never the token, the Claude account it
  belongs to, or the label its person gave it.
- **Three levels, chosen twice.** `details` shows all of that; `readiness`
  only whether each member's sessions can run and when their token runs
  out; `off` nothing. The instance's admins choose the most a workspace's
  may see (`details` by default) and whether limit pauses are in it (on
  by default); each workspace chooses within it. Its members read what was
  chosen in the workspace's Settings → You, so nobody is watched without
  knowing it.
- **Limit pauses are the workspace's own.** A workspace's view counts the
  pauses of its own sessions only, not those a person's sessions had in
  their other workspaces on the same subscription.
- **Instance admins see everyone's only where the instance says so**
  (`claude_instance_view`, off by default), for instances that set seats
  with admin tokens.
- **Admins hear by email once a week by default**, a summary sent only
  when a member's token runs out within a month or stopped working; a
  workspace can choose an email for each, or none. People always hear of
  their own, 30 and 7 days before and at once when it stops working.
- **Claude's refusals end a token, not fctl's guesses.** The control plane
  reads Claude Code's `authentication_failed` in the events sessions
  already send, so agents and hosts need no update. Once Claude refused a
  token in as many sessions as the instance says (2 by default, 1 to 5)
  since it last answered it, fctl marks it ended. fctl doesn't check
  tokens that don't run: that would spend people's subscriptions on
  requests nobody asked for.
- **A token's date is known where someone says it.** `made_at` gives it
  (the app asks "When did you make it?", defaulting to just now);
  otherwise fctl counts the year from the connection and says "around".
  The same token set again keeps its date, so a sync that sets it on every
  run no longer moves it a year on.

## Consequences

- Admins of an organization's workspaces see, at a glance, who can't run
  sessions and why, and what to do about each; members see what they see.
- A workspace's admins on app.fctl.sh see plans and expiry dates of their
  members by default; the privacy policy says so, and the instance or the
  workspace can lower it.
- A token whose sessions all fail for another reason is never marked
  ended by mistake, and one that's marked ended by mistake works again
  when its person connects it again.
- Long-lived tokens connected before this have an estimated date until
  connected again with `made_at`.
