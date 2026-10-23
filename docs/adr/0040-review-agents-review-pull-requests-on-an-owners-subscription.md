# 0040: Review agents review pull requests on an owner's subscription, and fctl posts for them

2026-10-22 · accepted · outsiders' pull requests wait to be asked for since [0047](0047-review-agents-review-outsiders-pull-requests-when-asked.md)

## Context

People ran review agents from GitHub Actions with a limited access token
([0037](0037-access-tokens-and-sessions-limit-what-agents-do-on-github.md),
[Automation](../automation.md)): a workflow per repository, one session
per pull request that new commits message, an agent that comments
directly with the GitHub App's token. That works, but each project has to
set it up, nothing shows on the pull request while the agent works, the
sessions crowd the Agent Floor, and a session whose agent can comment can
also approve, close and edit pull requests, which a pull request's own
text can ask it to.

People asked for review agents built in: a project turns them on, and
each new commit of its pull requests gets a review that points out bugs,
security problems and breaks of the repository's rules, on the lines
they concern, without nitpicks, in one pass, with a check that passes
when there is nothing to say and stays neutral when there is. Someone has
to pay for the reviews: the person who turns them on.

## Decision

- **Per project, on an owner's subscription.** The project's owners (its
  workspace's admins among them) turn review agents on, and they run on
  the Claude subscription of whoever did, the operator. Another owner
  takes them over, onto their own; nobody puts them on someone else's,
  and the operator may always turn them off. Owners choose the model, the
  effort level and instructions for reviews, apart from the project's
  instructions for its other agents.
- **Every new commit, drafts too.** GitHub's `pull_request` deliveries
  (opened, reopened, pushed to, ready for review) start a review of the
  head of each open pull request in the repositories the project's agents
  may write to, where they could comment. A commit gets one review from
  deliveries; a newer one stops a review still under way. A review that
  started finishes when its pull request merges meanwhile. Merged, closed
  and queued ones start none.
- **A session of its own kind.** A review is a session, so it runs on the
  project's runners and image, its transcript shows the agent's work, and
  people stop it like any other. It takes no messages, stays off the
  Agent Floor and the session lists, and the project's Reviews tab lists
  it instead. Only hosts that know review sessions get them.
- **The agent reads; fctl posts.** The review agent's GitHub token only
  reads, and it gets none of the project's secrets, setup script, memory
  or claude.ai connectors. It hands what it found to the control plane
  (`fctl review submit`), which posts it, marks the earlier findings it
  says the new commits fixed (fctl's own threads alone), resolves their
  threads and completes the check, with a token that never leaves the
  control plane. So a pull request that steers the agent gets at most
  comments on itself: no approvals, no closed pull requests, no secrets,
  no notes for later agents.
- **A summary, then the findings.** What a review found goes up as a
  summary, a comment on the pull request (how many issues, the agent's
  sentence or two, findings on lines the diff doesn't show), then one
  review of the commit with the other findings on their lines and no
  text of its own. GitHub's mobile app shows a review's own text below its
  comments, so a summary in the review would come after the findings it
  sums up; as a comment of its own, posted first, it comes above them
  everywhere. Each later review says atop the earlier summaries how
  their issues stand, fixed or resolved, and hides those whose issues
  are all addressed as outdated. It tells a summary's findings by the
  review session that posted both, which fctl notes out of sight in
  each comment.
- **The control plane writes the review prompt.** It goes to the agent in
  the session's spec (`ReviewSpec::instructions`), after the sandbox's
  facts that fctl-agent adds, so it improves with each deploy, without
  waiting for hosts to update.
- **A check and a reaction.** The control plane shows the review on the
  pull request as it starts: an eyes reaction, and a check, `fctl
  review`, on the commit: queued, in progress, then passed with nothing to
  point out, neutral with findings, cancelled when it ended without
  posting. Never failed: acting on a review is up to the people.

## Consequences

- The GitHub App needs the Checks permission (read and write), which each
  installation's owners accept; without it reviews post, without checks,
  and the Reviews tab says why.
- Every new commit of every pull request costs the operator a review's
  tokens: with many contributors, or bots that push often, the owners may
  want fewer (choosing repositories and skipping drafts come later). A
  newer commit stops the review it makes useless.
- Pull requests from people who can't write to the repository get reviews
  too; their agent runs none of their code and has nothing worth taking,
  but their text reaches the operator's subscription.
- A review with findings is two items on the pull request, a comment
  and a review, so people who watch it hear of both. Reviews posted
  before summaries were comments of their own keep theirs in the review,
  and later reviews leave it as it is.
- Review sessions take numbers from their project's sequence, so the
  sessions list skips the numbers reviews took.
- Hosts from before review agents get no review sessions: a project whose
  runners all lag waits, saying so, until they update.
