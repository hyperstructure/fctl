# 0069: Releases start their sandbox image and Homebrew bump, as a GitHub App

2026-10-29 · accepted

## Context

[0012](0012-the-sandbox-image-lives-in-its-own-public-repository.md)
connected hyperstructure/sandbox to this repository through fctl.sh, not
tokens: the sandbox's Image workflow looked for new releases on fctl.sh
every hour, and the Release workflow started nothing there. A release's
image was to follow within the hour. GitHub starts scheduled workflows
late, though, hours late at times: on 2026-10-29 the hourly run came two
to six hours apart, 0.26.1 had no image half an hour after its release,
and 0.23.0 never got one, as 0.23.1 came out before the hourly run did.
Hosts that update themselves to a release start their sessions from
`latest`, the release before's image, until their own is out.

The Homebrew tap's daily run had the same problem, and since #164 the
Release workflow starts its brew bump with `HOMEBREW_TAP_TOKEN`: a
fine-grained token, tied to the person who made it, that expires. #164
noted that a GitHub App would avoid both. The organization has one since,
fctl-actions, with which the tap's bump opens and merges its pull requests.

Session #541 was asked to have the sandbox image built when fctl is
released, and offered fctl-actions for it.

## Decision

- **A Sandbox image job** in the Release workflow, after Publish, beside
  Homebrew tap. It runs the sandbox's Image workflow with the release's
  version (`gh workflow run image.yml -f version=X.Y.Z`) and waits for the
  run, failing when it fails: the release's run shows whether its image
  went out, and the person who started it hears of a failure. The Image
  workflow runs one at a time, and GitHub keeps only one run waiting, so a
  newer run cancels the release's while it waits; the job then starts it
  again, up to three times. The run took 5 to 7 minutes in the week
  before, which the release spends waiting for its standalone image
  anyway.
- **Both jobs get their tokens from the fctl-actions GitHub App**
  (`actions/create-github-app-token`), whose key is
  `FCTL_ACTIONS_PRIVATE_KEY` in the production environment, which only
  `main` may use. Each token covers one repository with Actions: write
  alone, which starts its workflows, and is revoked as the job ends. The
  app has Actions read and write beside the Contents and Pull requests it
  had for the tap, and is installed on the tap and on the sandbox.
  `HOMEBREW_TAP_TOKEN` goes.
- **The sandbox keeps deciding what to build and tag** from fctl.sh, and
  its hourly run stays, for a release whose own run failed or never
  started. Since the release's run starts within a minute of the release,
  it counts a release later than fctl.sh's `latest`, cached for a minute,
  as the newest, and gives fctl.sh two minutes to show the release
  ([hyperstructure/sandbox#11](https://github.com/hyperstructure/sandbox/pull/11)).
  A concurrency queue (`queue: max`) would keep the release's run from
  being cancelled, but actionlint, which checks the sandbox's workflows,
  doesn't know it yet, and GitHub refuses it beside `cancel-in-progress:
  true`, which the workflow sets for pull requests.

## Consequences

- A release's image is out minutes after the release, and hosts start
  sessions from `latest` only for those minutes.
- The repositories connect through a token too, which amends 0012: the
  Image workflow's file name and its `version` input are an interface the
  Release workflow relies on, as the sandbox's AGENTS.md says.
- A failed image fails the release's run although the release is out.
  Running the failed job again starts the Image workflow again, and a run
  that finds the image current publishes nothing.
- The production environment holds a key that can make tokens for
  everything the app may do where it is installed, writing the tap's and
  the sandbox's contents and pull requests too, and merging in the tap
  without a code owner. It already held the deploy key that pushes to
  `main` past its rulesets.
- Without the app's key, or its access, both jobs fail, and the tap's
  daily run and the sandbox's hourly one catch up, late.
