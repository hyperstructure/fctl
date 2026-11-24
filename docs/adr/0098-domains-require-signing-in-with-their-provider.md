# 0098: Domains require signing in with their provider

2026-11-23 · accepted

## Context

An organization that runs fctl for its people wants them to sign in the
way they sign in everywhere else: with the identity provider that holds
their accounts, under its two-step verification and device checks, unable
to sign in once it closes their account, with no fctl password to forget,
have phished or leave behind (#618). fctl signed people in with a password,
passkeys or Google, and nothing made Google the only way: someone at a
company that standardized on Google Workspace kept the password they set
before, and with it their access after Google closed their account.

Google is the provider organizations running their own instance use today,
and others (Okta, Microsoft Entra ID, Keycloak) are to follow. Google
vouches for an address only where it hosts it (`fctl_domain::google`); a
Google account with any other address, a personal Gmail one included, is
none of the organization's business.

## Decision

- **Each sign-in records how it was made**: `password`, `passkey` or the
  provider's id, and for a provider, the domain of the address it vouched
  for (`tokens.auth_method`, `auth_domain`). A CLI's sign-in takes it from
  the browser session that approved it, and an access token or an admin
  token from the sign-in that made it. A linked account keeps the domain
  its provider last vouched for (`identities.vouched_domain`).
- **Instance admins require a provider per domain**
  (`InstanceSettings::sign_in`): "acme.com and acme.dev: Google only". A
  rule covers every account with an address at one of its domains,
  connected addresses included. A covered account signs in only with the
  rule's provider as an address at one of its domains; passwords, other
  providers' and other addresses' sign-ins, and passkeys alone unless the
  rule allows them, stop working, those made before the rule too: every
  request checks its token's sign-in (`auth::authenticate`). People at
  other domains sign in as before.
- **Passkeys alone don't meet a rule unless it says so.** They're quick,
  but a passkey still signs in someone the provider closed out. They stay
  second factors and confirm sensitive actions.
- **The primary instance owner keeps every way in**, whatever the rules
  say: the instance's way in while the provider is down. No other named
  accounts are exempt.
- **No admin signs themselves out.** Saving rules that would cover the
  admin who saves them, when their sign-in doesn't meet them, is refused:
  they sign in with the provider first, which shows it works. Admin →
  Sign-in and `fctl admin sign-in require` show whom a change concerns
  before it saves, and fctl emails those whose password or passkeys stop
  working.
- **The provider's first sign-in moves people over.** At a domain whose
  rule makes it the way in, the provider's account links by the address it
  vouches for, whatever else the account has, unless another account of
  the provider at the rule's domains is linked already (the address may
  have been handed to someone new), or the account is the primary
  instance owner's. Once the sign-in is whole, its second factor
  included, the account's password, reset links and sign-ins made another
  way go. Elsewhere, linking by address stays limited to accounts with no
  way in at all.
- **Covered addresses stay.** People can't disconnect an address at a
  covered domain, which would let them sign in another way; an instance
  admin or an admin token can.
- **Accounts only from the directory** (`InstanceSettings::self_sign_up`,
  on by default): off, signing in makes an account only for an address
  with an allowlist entry of its own (an invitation's, or one an admin
  allowed); a domain's entry admits the accounts at its domain and makes
  none.
- **Google is one provider among others to come.** Its module checks what
  Google sent and hands what it says of the person to a provider-neutral
  `api::sign_in`, which links, signs in and signs up by the rules. Rules,
  tokens and identities name providers by id.
- **Rules are the instance's**, as its admins own its allowlist: a
  workspace of its own doesn't require a provider, and app.fctl.sh, where
  workspaces of many organizations share the instance, has none to set.

## Consequences

- An organization moves its people off passwords in one step: their next
  sign-in goes through its provider, and once the provider closes their
  account they can't sign in again, unless a rule lets their passkeys in.
  The sessions they have go on until they end, or an admin disables
  their account: signals from the provider that someone signed out are
  later work.
- Saving a rule deletes, at once, every browser session, CLI sign-in,
  access token and admin token of a covered person not made with its
  provider, those from before the columns existed included, so that none
  comes back if the rule goes: they sign in again, and make tokens again,
  once. A directory sync's admin token made by a covered admin another
  way stops with them.
- A password that went at someone's first sign-in with the provider
  doesn't come back when the rule goes: they sign in with the provider as
  before, or set a password from Settings or a reset link.
- An organization whose provider is down signs in only as its primary
  instance owner until it's back, or until they let the domain go.
- A provider an admin adds in D1 (an OpenID Connect issuer, with its own
  client, scopes and the domains it vouches for), a workspace's own
  requirement, session policies and the provider's signals that someone
  signed out are later changes on this base (#618).
