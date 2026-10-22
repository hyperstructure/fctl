# 0037: Access tokens and sessions limit what agents do on GitHub

2026-10-22 · accepted

## Context

A personal access token did everything its person could: start sessions in
every project they see, change projects' settings and secrets, invite
people, make more tokens. And every session's agent could push to the
repositories its project may write to: its GitHub token wrote contents,
pull requests, issues and workflows.

People want agents that do less. A review agent that a workflow starts for
each new pull request should read the code and the pull request, comment,
review and resolve review threads, and push nothing. A planner should read
everything and write nothing. And the token such a workflow holds, in a
place where others may get at it, shouldn't reach the rest of its
person's projects, their account or their other tokens.

GitHub offers no permission that comments but can't push: comments and
reviews take `pull_requests` or `issues` write, which can't push, but
resolving a review thread takes `contents` write, which can.

## Decision

- **Sessions say what their agent may do on GitHub:** push, comment or
  read (`GithubAccess`), within their project's setting
  (`ProjectSettings.github_access`) and the token that starts them. The
  session's GitHub tokens carry those permissions
  (`fctl_domain::repos::permissions`), so GitHub enforces the choice, not
  the agent: one that comments gets `pull_requests` and `issues` write and
  reads contents; one that reads gets one read-only token for everything.
  fctl-agent pushes nothing for them and tells Claude what it may do.
  A session keeps what it started with; lowering the project's setting
  reaches running sessions with their next token.
- **The control plane resolves review threads** for agents that comment
  (`fctl resolve`, an agent call), with a token of its own that writes the
  thread's repository and never leaves it, after checking that the thread
  is on a pull request of a repository the session may write to.
- **Access tokens may be limited** in what they do (read; run sessions;
  everything), where (some workspaces and projects) and in what their
  sessions' agents may do on GitHub (`fctl_domain::limits::Limits`). Their
  roles are capped to a viewer's or an editor's, and a session whose agent
  started able to do more than the token allows is read only to it, also
  once its project holds it lower, since its sandbox keeps its tokens until
  they expire. Only a token that runs sessions may hold its agents lower:
  one that does everything could change the settings that steer them
  (setup script, instructions, image, secrets). Limits are chosen from
  three questions rather than a list of scopes, so that a person can say
  what a token is for and read back what it does.
- **A limited token reaches projects and sessions only.** One rule in the
  router (`limits::allows`) refuses it everything else, so that no handler
  has to remember to; it never reaches the account, workspaces' people and
  settings, other tokens or administration. Tokens without limits, browser
  sessions and the CLI's sign-ins work as before.

## Consequences

- Review agents run from CI with a token that can't push code, start
  sessions elsewhere or make itself another token
  ([Automation](../automation.md)). Their sandboxes still get the project's
  secrets, and the code they review can try to steer them: they can comment
  what it asks, not push it.
- Resolving threads goes through the control plane, one GraphQL call to
  look the thread up and one to resolve it, on the installation's rate
  limit. Agents that push resolve them with their own token, as before.
- An agent that comments can still close or edit pull requests and issues,
  and approve pull requests, as GitHub's `pull_requests` and `issues` write
  allow. The prompt asks it to approve only when asked.
- Hosts and agents from before ignore a session's GitHub access: their
  agents try to push, and GitHub refuses. A control plane from before
  ignores a token's limits, so `fctl token` checks the answer and revokes a
  token made without them.
- The token list says what each token does, in a sentence the app and the
  CLI build from its limits.
