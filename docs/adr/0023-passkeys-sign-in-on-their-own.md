# 0023: Passkeys sign in on their own, and count as a second factor where it's required

2026-10-19 · accepted

## Context

[0022](0022-two-factor-sign-in-with-authenticator-apps.md) added
two-factor sign-in with authenticator apps. That makes accounts safer but
sign-ins slower: a password, then a six-digit code. Passkeys (WebAuthn,
discoverable credentials) are faster and safer at once: the device or
password manager keeps a key that signs in after a fingerprint, face or
screen lock, and it works only on the site it was made for, so phishing
gets nothing. Every current browser has them, and offers them in a form's
email field.

## Decision

- **A passkey signs in on its own.** The device verifies the person
  (`userVerification: "required"`), so a passkey is two factors in one: no
  password and no code follow it.
- **Where two-factor is required, a passkey counts as the second factor**:
  after a password or Google, it works in place of a code, and having one
  satisfies the requirement. People can meet it with a tap instead of an
  authenticator app, and the page that requires it offers a passkey first
  on devices that can keep one. Elsewhere, adding a passkey changes nothing
  about other sign-ins: a password alone still signs in, unless the person
  has an authenticator app, so nobody turns on two-factor without meaning
  to. With an authenticator app, the code step offers the passkey too.
- **No attestation.** fctl doesn't ask which authenticator made a passkey
  (`attestation: "none"`) and checks no attestation statement; the AAGUID
  only names the passkey after its provider. Keys: ES256, EdDSA, RS256
  (Windows Hello), the ones browsers make.
- **Parsing in fctl-domain, signatures with WebCrypto.** A small CBOR
  decoder and the WebAuthn checks are pure Rust with tests; WebCrypto in the
  Worker checks the signatures, as it does for Web Push, so the Worker
  gains no crypto code.
- **One-time challenges, found by what the browser signs.** Each ceremony's
  challenge (256 random bits) is kept by its SHA-256 for ten minutes and
  deleted when it's used; `clientDataJSON` names it, so no cookie or token
  travels with it. A counter that goes back refuses the sign-in.
- **Adding one takes proof.** A passkey is a way in that skips the password
  and the code, so adding one takes the password unless the browser signed
  in within 15 minutes (or, without a password, a code from the
  authenticator app), and emails the person.
- **The sign-in page fetches its options on load**, so that the browser can
  offer passkeys in the email field and the button can open the passkey
  sheet within the click, as Safari requires.

## Consequences

- Passkeys belong to the host name of `FCTL_PUBLIC_URL`; an instance that
  moves to another host name loses them, and people add new ones.
- Where a passkey is someone's only second factor, losing it (and every
  synced copy) means an admin's reset, which removes the authenticator app,
  the recovery codes and the passkeys. Recovery codes stay with the
  authenticator app; Settings suggests a second passkey on another device.
- The last passkey can't be removed while it is the account's only way in,
  or the second factor the instance requires.
- Signals keep providers' lists current in browsers that have them; in
  others, a removed passkey stays offered until the person deletes it, and
  using it says that it's gone.
