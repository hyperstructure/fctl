# 0031: Admins choose a GitHub account's repositories after signing in

2026-10-20 · accepted

## Context

[0029](0029-workspaces-connect-github-accounts-with-a-github-sign-in.md)
made connecting a GitHub account a GitHub sign-in: what GitHub shows the
workspace's admin decides what the workspace may get. But the sign-in
connected everything at once: every repository the admin may give, unless
they had typed the names of some before it. Installing the app from a
workspace ended the same way, and people who wanted only some repositories
had to type their names into a form after GitHub had just shown them the
list.

Nor did installing bring people back. GitHub sends them to the app's setup
URL only where the app has one, which app.fctl.sh's didn't, so they stayed
on GitHub's page of the installation. With one, the state that says which
workspace started the installation comes back only after an installation:
after a change to one ("Configure", then Save, to give the app more
repositories), GitHub drops it.

## Decision

- **The sign-in offers; the admin chooses.** What a GitHub sign-in shows
  the admin they may connect (`fctl_domain::github::offer`: every
  repository of their own account, or of an organization whose every
  repository the app reaches and they administer all of; otherwise the
  ones they administer) waits for them in D1 (`github_offers`), for ten
  minutes, as long as the sign-in's link works. The workspace's GitHub tab
  lists it, with what the workspace works in there now checked, else all of
  it, and connects what they choose (`POST
  /api/v1/workspaces/{id}/github/offers/{account}`), which must come from
  the offer (`fctl_domain::github::choose`). The user token is still used
  once and revoked at the sign-in; only the names of the repositories the
  admin may connect wait, and only they see them.
- **No names to type.** A sign-in that names no account offers every
  installation of the app GitHub shows the admin (at most 20), so the
  GitHub tab's form is one button. Naming repositories before signing in
  (`fctl workspaces connect acme --repo web`) still connects them at once,
  as far as GitHub says the admin may (`fctl_domain::github::grant`).
- **Installing comes back to where it started.** The link to install the
  app goes through the control plane (`/api/v1/github/install`), which keeps
  its signed state in a cookie (`__Host-fctl-github-install`) for the hour
  it works, besides passing it to GitHub. The setup URL takes the state
  from GitHub, or else from the cookie, and goes on to the GitHub sign-in
  for the installation GitHub names, so an installation or a change to one
  started from a workspace ends on its GitHub tab, to choose. A setup URL
  reached without either (an installation changed on GitHub itself) goes to
  the GitHub tab of the person's workspace that uses the installation, or of
  the only one they're an admin of, without signing in to GitHub: anyone
  can send that URL, so it only asks them to connect.
- **The GitHub App needs its setup URL**, `…/api/v1/github/installed`, with
  Redirect on update, as the self-hosting guide already said; app.fctl.sh's
  admins set it on its app.

## Alternatives

- **Connect everything, then let the admin narrow it.** No offer to keep,
  but the workspace would hold, for a while, repositories nobody chose for
  it, and its people's agents could use them meanwhile.
- **Keep the offer in a signed cookie, as the sign-in's state is.**
  Browsers keep 4 KB a cookie, about a hundred repository names; an
  organization's admin can be offered thousands.
- **List the installation's repositories with the app's own token
  instead of the user's.** It would show private repositories the admin
  can't see, and repositories they don't administer as if they could give
  them.
- **Trust the setup URL's `installation_id` without a sign-in.** As 0029
  says, anyone can send it; the sign-in is what proves anything.

## Consequences

- Installing the app, giving it more repositories or connecting an account
  ends with a choice on the workspace's GitHub tab, not a connection made
  for the admin. Each connected account has a button that signs in again to
  choose anew.
- `fctl workspaces connect` without `--repo` ends in the browser, where the
  admin chooses; its account is optional.
- D1 keeps an admin's offers for ten minutes or until they choose; the
  hourly job deletes those that ran out. app.fctl.sh's privacy policy says
  so, and names the new cookie.
- `Refusal::TooMany` is gone: an admin of more repositories than a
  connection names (100) chooses some of them.
