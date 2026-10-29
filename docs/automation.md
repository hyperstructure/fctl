# Automation

Agents start when you ask, at the times you set, when someone delegates
a ticket to fctl in Linear, and when your CI, a bot or another tool asks:
schedules start sessions on their own, and an access token lets a
workflow start them, limited to what it is for. This page shows how to
set up schedules, how to delegate Linear tickets, how to start agents from
other tools, such as another issue tracker, how to make a token, how to
run review agents on your pull requests from GitHub Actions, and other
uses: triaging issues, nightly maintenance, planning, dashboards.

## Schedules

A schedule is a task that fctl starts a session for on its own: once at a
set time, or again and again, every hour, day, weekday, week or month, or
on any cron expression, in a time zone you choose; or only when asked, by
you or another tool ([below](#start-agents-from-other-tools)). Set one up
under the project's **Schedules** tab, which offers ideas to start from
(dependency updates, triaging new issues, tidying the project's memory,
learning from the week's reviews), or with `fctl schedules add`;
**Schedule**, beside Start in the project's task box, and in the Agent
Floor's in a wide window, turns what you wrote there into one.

- **Each run is a session.** It is named after the schedule, works on a
  branch of its own and shows in the project's sessions with the
  schedule's mark; its page links to the schedule, and the schedule lists
  what each of its runs started. The agent gets the task, then a note
  from fctl: which schedule started it and when, its previous run, and
  that nobody watches it start, so that it works without waiting for
  answers and says plainly what only a person can decide.
- **On whose subscription.** A schedule runs on the Claude subscription of
  the person who set it up, its owner, who follows its sessions as they
  follow the ones they start (unless they turned that off in Settings). A
  change by someone else, as one of the project's maintainers or owners,
  runs it on theirs from then on: nobody puts work on someone else's
  subscription. Running one now, from the tab or with `fctl schedules
  run`, starts a session on your own.
- **Runs don't pile up.** A run is skipped while the schedule's last
  session still works on its task, and while its owner's plan is at its
  limit, until the limit resets; a schedule that runs once waits for the
  reset instead. The schedule says so, and why, in its runs.
- **When its owner can't run it.** Once they can no longer start sessions
  in the project, their account is gone or disabled, or they have no
  Claude connection, fctl pauses the schedule at its next run and says
  why; resuming it runs it on the subscription of whoever resumes it.
- **What it may do.** Its sessions start with the model, effort level and
  GitHub access it sets, or the project's, and never more on GitHub than
  the project allows now. Schedules run at most every 15 minutes, and a
  project keeps at most 50.

```sh
fctl schedules add "Weekly dependencies" -p web --cron "0 9 * * 1" \
  -m "Update the dependencies that have new patch or minor releases, run the tests and open a pull request."
fctl schedules -p web                       # when each runs next, and what its last run did
fctl schedules run "Weekly dependencies" -p web
```

## Delegate tickets in Linear

Teams that track their work in Linear hand tickets to fctl there, and
fctl starts the sessions itself, without a schedule, a token or a relay:

1. **Connect Linear** on the workspace's **Linear** tab, or with `fctl
   workspaces linear connect`: an admin of the workspace who is an admin
   in Linear installs fctl's app there and chooses the teams it sees.
2. **Choose the teams** in the project's **Tickets** tab, or with `fctl
   projects set -p web --tickets linear --linear-team ENG` (`ENG,DES`
   for several), and if you like one Linear project of each. Its agents
   then read and write those teams' tickets with `fctl ticket`, and no
   other tracker, and file new ones in the first team unless one belongs
   in another.
3. **Link your Linear account**, once, in Settings or with `fctl account
   linear connect`. Until you do, what you delegate waits an hour for you,
   with a button in Linear that links it.

Then, in Linear, assign an issue to fctl, which makes fctl its delegate
while you stay its assignee, or mention fctl in a comment. fctl starts a
session of the project that takes the issue's team, on your Claude
subscription, and the issue's agent session shows it working: a first
thought, its steps, its pull requests, and each turn's last message, or
its question. Reply there to answer it or to ask for more, and stop it
there to interrupt it. Delegate a ticket with the `explore` label for a
plan first: the agent finds out what it needs and replies with what it
found, and changes nothing until someone asks. The ticket moves to In
Progress as its session starts, to In Review when a turn ends with the
session's pull request up, and to Done once the pull request that fixes
it merges. The session's branch carries its key, so with Linear's GitHub
integration the pull request links to the ticket too.

Only people who may start sessions in the project start them this way,
on their own subscription; a ticket from someone else, or for a team no
project takes, gets an answer in Linear that says why
([Architecture](architecture.md#linear)).

## Start agents from other tools

Any tool that can make a web request can start an agent: an issue
tracker's automation, a chat bot, a deploy pipeline, a script on a
server. Point it at a schedule whose task says what to do in that case,
and send what the tool knows of why with it, such as an issue's title and
text. Each request starts a session with the schedule's task and what
came with it, on the Claude subscription of the token's person, and the
schedule lists it as a run by a tool.

1. **A schedule for the case.** One that runs **when asked** has no time
   of its own, since the tool decides when it runs: choose When asked in
   its form, or `fctl schedules add "Triage an issue" -p web --when-asked
   --github comment -m "Triage the issue that comes with this: label it,
   say what is missing to act on it, link duplicates and point at the code
   it concerns."` Its details show the request below, with its id, ready
   to copy (any schedule can be run this way).
2. **A token that runs sessions.** `fctl token "Issue triage" --access run
   -p web --github comment --days 90`.
3. **The request.** The tool posts to the schedule's runs, with what it
   knows in `context` (at most 20,000 characters):

   ```sh
   curl -H "Authorization: Bearer $FCTL_TOKEN" -H "Content-Type: application/json" \
     -d '{"context": "ENG-412: The login page hangs on Safari\n\nSteps: …"}' \
     https://app.fctl.sh/api/v1/schedules/sch_01JC…/runs
   ```

   Or from a shell: `fctl schedules run sch_01JC… --context-file -`, with
   the text on stdin.

What a tool sends comes from outside fctl, often from people who can't
instruct your agents, such as whoever filed an issue. So the agent gets
it after the task, marked as information for the task and not as
instructions. Its token can't do more than run sessions in the projects
it names, and its sessions' agents can't do more on GitHub than it allows.
People and tools may run one schedule 20 times an hour, and pausing it
stops them all. Tools that send webhooks without a way to add a header, as
some issue trackers do, need a small relay (a serverless function, or a
no-code tool such as Zapier, Make or n8n) that adds it and picks out the
fields to send.

## Access tokens

Make one in **Settings → Access tokens**, or with the CLI:

```sh
fctl token "PR reviews" --access run -p web --github comment --days 90
```

It prints the token once. Three questions limit it:

| Question | Choices | `fctl token` |
|---|---|---|
| What it can do | **Run sessions**: start sessions, message their agents, stop and resume the ones it started, keep projects' memory. **Read**: read projects and sessions, change nothing. **Everything**: all you can do in its projects. | `--access run`, `read` or `full` |
| Where | All your projects, or some projects, or every project of some workspaces (those made later too) | `-p web` and `-w acme`, repeated |
| What its sessions' agents may do on GitHub | Push, comment or read (below), for a token that runs sessions | `--github push`, `comment` or `read` |

A token with any limit reaches projects and sessions only: never your
account, your workspaces' people and settings, your Claude connections or
your other tokens, so it can't make itself a token without limits. Only a
token that runs sessions holds their agents to commenting or reading: one
that does everything could change its projects' settings, and so what
every agent there does. It
works as you, within its limits and never beyond your own roles, until it
expires or you revoke it. A token without limits, which `fctl token NAME`
makes when you ask for none, does everything you can, as you do when signed
in: keep those to your own machine.

```sh
fctl tokens                      # your tokens and what each does
fctl tokens revoke "PR reviews"  # by name or id, at once
FCTL_TOKEN=fctl_pat_… fctl whoami   # what the token in use does
```

Sessions a limited token starts are yours, in your lists, on your Claude
subscription, but they don't notify you: follow the ones you want with the
bell on their page or `fctl follow`.

## What agents may do on GitHub

Each session's agent pushes, comments or only reads, in the repositories
its project may write to:

| | Its GitHub token | The agent |
|---|---|---|
| **Push** | writes contents, pull requests, issues and workflows | commits, pushes its branch and opens pull requests, as agents always did |
| **Comment** | writes pull requests and issues, reads contents | reviews, comments, labels and resolves review threads; can't push or merge |
| **Read** | reads | reads the code, pull requests, issues and CI results; writes nothing |

A session asks for one as it starts (the list beside the model in the
task box, `fctl run --github comment`), at most what its project allows
(the project's **Agents on GitHub** setting, `fctl projects set --github
comment`) and what the token that starts it allows; it gets as much as
they allow when it doesn't ask. GitHub enforces it: the agent's tokens
can't do more. Its page says what its agent may do.

GitHub lets only tokens that can push resolve review threads, so an agent
that comments resolves them through fctl, `fctl resolve PRRT_…` in its
sandbox, which its `~/AGENTS.md` explains. An agent that comments can
still edit, close and approve pull requests and issues, as GitHub's
permissions for comments allow: it is asked to approve only when its task
says so.

## Review pull requests from GitHub Actions

Projects have review agents built in: turn them on under the project's
**Reviews** tab, or with `fctl review settings --on`, and each new commit
of a pull request gets a review, with a check on the commit (outsiders'
pull requests when someone asks for it); when an fctl
agent opened the pull request, its session gets the findings to act on
([Architecture](architecture.md#review-agents)). Follow-ups bring an fctl
agent back to its pull request without a workflow too, when someone
mentions `@fctl` in a comment, a check fails, it conflicts with its base
branch or it merges ([Architecture](architecture.md#follow-ups)). A workflow of your own
suits reviews those don't make: ones that run the tests, or that start on
a label. Here one reads each new pull request, runs its tests if that
helps, and posts a review, for the project `web`, on app.fctl.sh:

1. **A token.** Make one that runs sessions in `web` alone, whose agents
   comment:

   ```sh
   fctl token "web PR reviews" --access run -p web --github comment --days 90
   ```

2. **A secret.** In the repository's **Settings → Secrets and variables →
   Actions**, add it as `FCTL_TOKEN`.

3. **A workflow.** `.github/workflows/review.yml`:

   ```yaml
   name: Review
   on:
     pull_request:
       types: [opened, ready_for_review, synchronize]

   jobs:
     review:
       if: ${{ !github.event.pull_request.draft }}
       runs-on: ubuntu-latest
       steps:
         - name: Install fctl
           run: |
             curl -fsSL https://fctl.sh/install | FCTL_NO_COMPLETIONS=1 sh
             echo "$HOME/.local/bin" >> "$GITHUB_PATH"
         - name: Start or update the review
           env:
             FCTL_SERVER: https://app.fctl.sh
             FCTL_TOKEN: ${{ secrets.FCTL_TOKEN }}
             PR: ${{ github.event.pull_request.number }}
             REPO: ${{ github.repository }}
           run: |
             # One session per pull request, found by its branch.
             branch="review/pr-$PR"
             session=$(fctl sessions -p web --limit 200 --json |
               jq -r --arg b "$branch" '[.sessions[] | select(.branch == $b)][0].id // empty')
             if [ -n "$session" ]; then
               fctl send "$session" "New commits on #$PR. Review what changed since your last review, and resolve your review threads they address."
             else
               fctl run -p web --github comment --detach --branch "$branch" \
                 --title "Review #$PR" \
                 "Review pull request #$PR of $REPO. Read its description and diff, check it out and run the tests where that helps, and post one review with gh pr review --comment: what is wrong or risky, with suggested changes on the lines they concern. Approve nothing."
             fi
   ```

   Pass what comes from the pull request, such as its title, through `env`
   rather than `${{ }}` in the script, where it could run as a command.

4. **The review.** Each pull request gets a session in `web`, which shows
   in your sessions as **Comments only**. Its agent works on one of your
   runners, on your Claude subscription, and posts the review as the
   GitHub App's bot. New commits message the same session, which reviews
   what changed and resolves the threads they address.

What to keep in mind:

- **Pull requests from forks.** GitHub gives `pull_request` workflows from
  forks no secrets, so their pull requests get no review. On
  `pull_request_target` they get secrets, and then whoever opens a pull
  request would start an agent on your subscription and runners: start
  those reviews on a label a maintainer adds (`pull_request_target` with
  `types: [labeled]` and `if: github.event.label.name == 'review'`). The
  workflow runs none of the pull request's code either way; the agent
  checks it out in its sandbox.
- **What the agent reads can steer it.** A pull request can ask the agent
  to do something else. It can comment what it is asked, not push it, and
  its token can't reach your other projects. Its sandbox has the project's
  secrets, as every session's does, and it can add to the project's
  memory, which later agents start with, those that push too: review code
  from people you don't trust in a project whose secrets they may see, or
  that has none, and keeps no memory (`fctl projects set --project-memory
  off`).
- **One project, an expiry.** Keep the token to the projects it serves and
  let it expire; revoke it in Settings → Access tokens, or with `fctl
  tokens revoke`, as soon as it may have leaked.

## Other uses

- **Triage new issues.** On `issues: types: [opened]`, start a session
  that comments: `fctl run -p web --github comment --detach "Triage issue
  #$ISSUE: label it, say what is missing to act on it, and point at the
  code it concerns."`
- **Nightly maintenance.** A schedule does it without a workflow
  ([Schedules](#schedules)): `fctl schedules add "Nightly dependencies"
  -p web --cron "0 3 * * *" -m "Update the dependencies that have patch
  releases and open a pull request."`
- **Plan before building.** Choose **Read only** in the task box, or `fctl
  run --github read`, and ask for a plan: the agent reads the code, issues
  and pull requests and answers in the session, writing nothing to
  GitHub. A project whose agents should only ever read or comment says so
  in its settings.
- **Dashboards and reports.** A token that reads (`--access read`) lists
  sessions, transcripts, retrospectives and project memory, and starts
  nothing: `fctl sessions --active --json`, `fctl retro list -p web --json`,
  `fctl logs web#12 --json`.
- **Your own tools.** The API takes the token as a bearer token:

  ```sh
  curl -H "Authorization: Bearer $FCTL_TOKEN" -H "Content-Type: application/json" \
    -d '{"prompt": "Review pull request #12", "github_access": "comment"}' \
    https://app.fctl.sh/api/v1/projects/web/sessions
  ```

  [Architecture](architecture.md#access-tokens) says what a limited token
  reaches, and [the CLI](cli.md) lists every command.
