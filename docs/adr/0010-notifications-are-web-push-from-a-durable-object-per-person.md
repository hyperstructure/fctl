# 0010: Notifications are Web Push, sent from a Durable Object per person

2026-10-18 · accepted

## Context

People start sessions and go do something else. Nothing told them when a
session finished its turn and waited for a reply, paused on a usage limit or
failed: they had to keep the app open and look
([#4](https://github.com/hyperstructure/fctl/issues/4)). Several sessions
often change at once, when parallel tasks finish together or a project's
sessions fail the same way, and a notification for each would be noise.

## Decision

- **Web Push (RFC 8030) to the browsers people choose**, with message
  encryption (RFC 8291) and VAPID (RFC 8292), sent by the control plane
  itself. No push provider account, no server key, nothing for self-hosters
  to configure, and the web app is the client everywhere, including iPhones
  and iPads, where Safari offers web push to sites on the Home Screen. Email
  is a separate decision.
- **The instance makes its VAPID key** on first use, with WebCrypto, and
  keeps it in D1's `settings`, sealed with the master key like every other
  secret. WebCrypto does ECDH and ECDSA in the Worker; the rest of the
  encryption is pure Rust in `fctl_domain::webpush`, tested against the
  RFC's example.
- **Following decides who hears about a session.** Starting or resuming one
  follows it (a setting turns that off); anyone who can see a session can
  follow or unfollow it. Access is checked when a notification goes out, so
  losing access to a project stops its notifications.
- **SessionDO decides that something happened**: a finished turn
  (`running` → `idle`), a pause or a failure. It leaves out people who can
  see the session right now (the web app says when its tab is hidden) and
  whoever interrupted the turn.
- **A NotifyDO per person decides when it goes out.** It gathers a burst for
  ten seconds, sends at most one notification a minute, makes several into
  one digest, and leaves out sessions that changed again meanwhile. An alarm
  does the timing, so nothing waits in a Worker request, and the object
  deletes what it stored once it is sent.
- **A browser's subscription belongs to the sign-in that made it**
  (`push_subscriptions.token_id`, deleted with the token), so signing out of
  a browser stops its notifications, and another account signing in there
  does not inherit them. The web app never brings back a subscription the
  server dropped: a browser someone removed stays removed.

## Consequences

- Notifications reach browsers that are closed, as long as the browser runs;
  a phone gets them through its OS. Push services see when a message goes
  and how big it is, not what it says; fctl.sh's privacy policy names them.
- The control plane posts to whatever endpoint a browser gives it, so it
  only accepts `https://` addresses of named hosts (development allows
  local ones) and at most 20 browsers a person.
- A notification can lag what happened by up to a minute, which is what
  keeps a burst to one. People who watch a session get none for it, so the
  delay matters little.
- Rotating the master key means sealing the VAPID key again, as with stored
  credentials. If it is lost, the instance makes a new one and the web app
  subscribes browsers again with it when they next open the app; until
  then their old subscriptions fail.
