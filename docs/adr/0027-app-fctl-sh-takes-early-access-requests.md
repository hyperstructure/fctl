# 0027: app.fctl.sh takes requests for early access

2026-10-20 · accepted

## Context

app.fctl.sh is invitation-only: an admin adds someone's address to the
allowlist and hands them an invitation link. Someone who hears of fctl and
opens the app finds a sign-in page with nothing for them but "ask an
admin", and no admin to ask. hyperstructure wants to let people in a few at
a time, from a list of the people who asked, and to hand each one their
link: app.fctl.sh sends no email yet.

Self-hosted instances are a team's own, and their people are invited by
whoever runs them: a list strangers can join has no place there.

## Decision

- **A deployment turns it on.** `early_access = true` in an instance's file
  in `deploy/instances` sets `FCTL_EARLY_ACCESS`, and `InstanceInfo`'s
  `features.early_access` tells the app. app.fctl.sh's file turns it on and
  the example's doesn't; the standalone server doesn't take the variable.
  `cargo xtask dev` turns it on, as app.fctl.sh, so that it can be tried.
- **An address, nothing else.** The page asks for an email address only,
  and D1 keeps it with when it first and last asked (`access_requests`):
  no name, IP address or browser. The rate limit per client address that
  sign-ups have applies.
- **The same answer for everyone.** Whether the address asked before, was
  invited or has an account, the request succeeds, so that it tells
  nobody who uses the instance. An address with an account isn't
  recorded. The browser that asked remembers it, so that the page can say
  it is on the list.
- **Inviting is the invitation there is.** Admins invite from the list with
  the instance invitation (`POST /api/v1/admin/invites`), which allows the
  address and makes a link that works once, for 14 days, and is emailed
  where email is set up. The list shows the link in place, to copy or to
  open in the admin's mail app, and any invitation to the address marks its
  request invited. No new kind of invitation, token or sign-up.
- **Requests don't stay.** One goes once its address has an account, when
  an admin removes it, or a year after it last asked.

## Consequences

- Anyone can add addresses to app.fctl.sh's list, as many as the rate
  limit lets them, and one can be someone else's: the admins see the
  addresses, and an invitation reaches only the address's owner, so a
  stranger's request gets them nothing. The list shows the first 500 and
  how many there are in all.
- The list is the admins' to work through: nothing tells them when
  someone asks but the count on the tab.
- [The privacy policy](https://fctl.sh/privacy) says what app.fctl.sh keeps
  for a request, who sees it and for how long, in
  hyperstructure/fctl-site.
- A self-hosted instance on Cloudflare could turn it on in its file; the
  page then speaks of early access to that instance.
