# 0022: Two-factor sign-in with authenticator apps

2026-10-18 · accepted

## Context

An fctl account holds a Claude subscription, starts agents that push to
the project's repositories, and reads its secrets. Until now a password was
enough to sign in, and a password reset link enough to take an account
over. People asked for a second factor that they can turn on themselves,
and that admins can require of everyone on an instance. Google sign-in has
its own second factor, two-step verification, so asking for another after
it should be a choice, not the default.

## Decision

- **The second factor is an authenticator app (TOTP, RFC 6238)**, with
  HMAC-SHA1, six digits and 30-second steps: the parameters every app
  supports, so the link leaves them out and no app misreads it. Codes of
  the time step before and after now count, for phones whose clocks are off,
  and each code works once. Ten recovery codes, each good once, stand in
  for a lost phone. SMS and email codes are not offered: SIM swaps and
  inbox access defeat them, and fctl may have no email set up.
- **A sign-in that asks for it starts no session.** It leaves a challenge
  for ten minutes, in D1 by its hash and in the browser as an `HttpOnly`
  cookie (`__Host-fctl-2fa`), so the second step works the same after a
  password, after Google's redirect and after a reset link, and survives a
  reload. The second step only takes codes: what the first step proved
  (the password, Google) is not asked again.
- **A reset link doesn't change a password by itself** when the account has
  two-factor: the new password waits in the challenge and takes effect with
  the code. Otherwise whoever reads the person's email could lock them out.
- **Google sign-ins ask for it when the person chose that or the instance's
  admins did** (`two_factor_google`). Otherwise Google's own sign-in is
  enough, as before.
- **Admins require it per instance** (`require_two_factor`), of every
  account that signs in with a password, and of every account once Google
  sign-ins ask too: an account that signs in only with Google, which never
  asks for a code, gains nothing from setting one up. An account it is
  required of that has none can only set it up. The control plane enforces
  that on every request, from browser sessions and CLI and access tokens
  alike (`two_factor_setup_required`), and reads the instance's settings in
  the query that resolves the token, so the check adds no round trip. There
  is no grace period: setting it up takes a minute, on the page the person
  sees next.
- **The CLI signs in through the browser** (the device flow), so it asks for
  no codes itself; it manages two-factor (`fctl account two-factor`) and
  prints the QR code in the terminal.
- **Wrong codes cost time.** Five in a row make codes from the app wait a
  minute, then twice as long after each further one, up to an hour: at most
  a few dozen guesses a day at a million codes, whatever the rate limits
  let through. Recovery codes, about 43 bits each, can't be guessed within
  the rate limits, so they still work during the wait: someone who has only
  the password can't keep the person out. The first wait emails the person,
  since whoever it was knew their password.
- **Changes take proof.** Replacing the app, turning two-factor off and new
  recovery codes take a current code (from the app, or a recovery code), so
  a stolen session alone can't take two-factor over. Setting up an app also
  takes the password, unless the browser signed in within 15 minutes; a CLI
  or access token always counts as old, since one can be made from a
  browser session that signed in long ago.

## Consequences

- Setting up an app needs the master key, which seals its secret.
  Requiring two-factor refuses to turn on without one.
- A person who loses their phone and their recovery codes asks an admin,
  who resets their two-factor (`fctl admin reset-two-factor`); owners'
  only an owner can reset. When email is set up, the person hears of it,
  and of every other change to their two-factor sign-in.
- Someone who has the password of an account that hasn't set two-factor up
  yet can set it up first. The email that follows and the audit log show
  it, and an admin can reset it.
- Personal access tokens and CLI tokens keep working without codes once
  issued: they are what scripts use. Requiring two-factor stops them only
  for accounts that haven't set it up.
- Older CLIs show `two_factor_setup_required` as an HTTP 403 with its
  message, since they don't know the error code.
