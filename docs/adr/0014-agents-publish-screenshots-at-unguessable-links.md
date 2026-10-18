# 0014: Agents publish screenshots at unguessable links

2026-10-18 · accepted

## Context

Pull requests that change how something looks show it with screenshots,
and agents take them. `gh pr create`, `gh pr edit` and `gh pr comment`
attach images with `--attach`, the way dragging them into GitHub's editor
does, but GitHub takes those uploads only with a person's token: a session
has the GitHub App's installation token, so `--attach` fails with
"unsupported authentication type", and the upload endpoint answers it with
a 404 ([#40](https://github.com/hyperstructure/fctl/issues/40)). The only
other way to show an image in a private repository's pull request is to
commit it, and screenshots are never committed. Pull requests described
their screenshots in words.

The issue weighed three ways: uploading as the person who started the
session (a user-to-server token kept for them, which GitHub may not accept
either), a machine account's token as a project secret (a long-lived token
in every sandbox), and hosting the images on fctl.

## Decision

- **fctl hosts them.** `fctl publish shot.png` in the sandbox sends the
  image to fctl-agent over the session socket; the host uploads it with its
  runner token; the control plane keeps it in the project's FilesObject,
  like attachments ([ADR 0008](0008-attachments-in-a-durable-object-per-project.md)),
  and answers with a link that GitHub's image proxy can fetch. The agent
  puts Markdown with the link in the pull request.
- **The link is the key.** It holds a token of 256 random bits and is not
  signed in: anyone with it can open the image. D1 keeps only the token's
  SHA-256 (`published_files`), as for every other secret. Responses are
  `nosniff`, sandboxed by their Content-Security-Policy, `noindex`, and
  `no-cache`: caches, GitHub's image proxy among them, ask again each time,
  so a deleted or expired image stops showing.
- **Only images GitHub shows.** PNG, JPEG, GIF and WebP, as their bytes
  say, up to 5 MB, the most GitHub's image proxy fetches; at most 100 per
  session. Nothing a browser would run.
- **They expire.** Links work for 30 days unless the project chooses 1 to
  60 days (`screenshot_days`); then the FilesObject's alarm deletes the
  bytes and the hourly cron trigger the row. People who manage the session
  delete one sooner, from the transcript or with `fctl unpublish`.
- **Off twice over until people turn it on.** The instance's admins allow
  it (`InstanceSettings.publish_screenshots`, Admin → Settings) and each
  project's maintainers turn it on (`publish_screenshots`, the project's
  settings). Both are checked when a screenshot is published and whenever a
  link is opened: turning either off takes the screenshots offline.
- **Try GitHub first.** `~/AGENTS.md` tells agents to attach with `gh …
  --attach` where that works and to publish when GitHub refuses, so a
  session that someday has a token GitHub accepts uses GitHub.
- **The transcript says what went out.** fctl-agent adds a
  `screenshot_published` notice with the file, so the people following the
  session see each image and its link. It is a notice, not a new event
  kind: CLIs from before it show the message, which has the link, where a
  new kind would fail to parse.

## Consequences

- Instances need nothing new: the FilesObject and D1 hold the screenshots.
  The links point at the instance (`/api/v1/published/…`), so a self-hosted
  one serves its own.
- Published screenshots are public in the way unlisted links are:
  whoever can read the pull request, or is sent the link, can open them
  until they expire. fctl.sh's privacy policy says so.
- Old pull requests show broken images once their screenshots expire. That
  is the point of expiring them; the session's transcript keeps saying what
  was published.
- The setting reaches agents only through hosts that know it, since a host
  rebuilds the spec it passes on: sessions on older hosts don't offer `fctl
  publish`, and their agents need no fallback.
- Agents publish through fctl, not as a person, so pull request
  descriptions are edited by the GitHub App's bot as before. When sessions
  get a token GitHub takes uploads from, `--attach` wins without a change
  here.
