# fctl for agents

This page is for agents, scripts and other programs that drive fctl: a
coding agent that hands work to fctl, a personal agent that works for
someone, a CI job, a bot, an issue tracker's automation. It says how to
sign in, with or without a person at hand, which commands return and which
keep running, how to wait for a session and read what its agent did, what
to avoid, and how to set up the common flows. The last part is about the
other side: what an agent working in an fctl session can do in its
sandbox. [llms.txt](https://fctl.sh/llms.txt) sums all of this up in one
file, with fctl's vocabulary.

Everything here works the same against app.fctl.sh and an instance of your
own: `FCTL_SERVER` says which. [The CLI](cli.md) lists every command, and
[the HTTP API](api.md) has the requests behind them.

## Signing in

A program uses an access token that a person made for it, limited to what
it is for, in `FCTL_TOKEN`:

```sh
fctl token "Nightly triage" --access run -p my-app --github comment --days 30   # a person runs this
export FCTL_SERVER=https://app.fctl.sh FCTL_TOKEN=fctl_pat_…
fctl whoami                      # whose it is, and what it may do
```

- `--access run` starts sessions, messages and interrupts their agents,
  stops and resumes the sessions it started, and keeps projects' memory;
  `read` only reads; `full` does whatever its person may in its projects.
  `-p` and `-w` keep it to some projects or workspaces, `--github` caps
  what its sessions' agents may do on GitHub, and `--days` makes it
  expire. A token with any limit never reaches the account, workspaces'
  people and settings, Claude connections or other tokens, so it can't
  make itself another. One without limits does everything its person
  can: keep those on people's own machines.
- `fctl token` prints the token once. Making one is a sensitive action:
  where the instance asks people to confirm it's them, it asks in the
  terminal, so a person makes it, not the program.
- Sessions a token starts run as its person, on their Claude subscription,
  and don't notify them; `fctl follow` does.
- `fctl login` signs a person in through the browser (`--no-browser`
  prints the link to open elsewhere). It needs the person: don't use a
  person's sign-in for unattended work.
- `FCTL_SERVER` and `FCTL_TOKEN` take precedence over what `fctl login`
  saved in `~/.config/fctl/` (`FCTL_CONFIG_DIR` moves that). `--server`
  takes precedence over both.
- Revoke a token that may have leaked at once: `fctl tokens revoke NAME`
  or Settings → Access tokens.

### From a personal agent

A personal agent works for one person across their apps and accounts, as
[Grok Bot](https://x.ai/bot),
[Claude Duet](https://claude.com/product/claude-duet),
[Meta Muse](https://ai.meta.com/muse/), [Wabi](https://wabi.ai) and
OpenAI's [ChatGPT Dots](https://chatgpt.com/features/dots) do. Its person
is there to answer, so before it signs in to fctl, it asks them which of
two kinds of access they want to give it, unless their request already
says:

- **A full sign-in**: the agent can do anything the person can in fctl,
  in every workspace and project, with their account, Claude connections
  and access tokens. It runs `fctl login app.fctl.sh --no-browser` (or
  their own instance's address), which prints a link and a code, then
  waits up to ten minutes and returns once the person approves. So the
  agent runs it in the background, or wherever it reads the output before
  the command returns, and without `--json`, with which it prints neither
  link nor code. It shows the person the link and the code: they open the
  link, check that the page shows the same code, and approve. The sign-in
  shows in the person's Settings → Devices, where revoking it signs the
  agent out at once.
- **A token for one project**: the agent can do in that project what an
  editor does: start sessions and steer their agents, run its schedules,
  keep its memory, and read its sessions and retrospectives. It reaches
  nothing outside the project: not the person's other projects, their
  workspaces' people, settings and runners, their account or their other
  tokens. The person makes the token, in Settings → Access tokens (New
  access token: Run sessions, Only some, the project, an expiry) or in a
  terminal, and gives it to the agent, which keeps it in `FCTL_TOKEN` and
  doesn't repeat it in the conversation:

  ```sh
  fctl token "Personal agent" --access run -p xyz123 --days 30   # the person runs this
  export FCTL_SERVER=https://app.fctl.sh FCTL_TOKEN=fctl_pat_…
  fctl whoami                      # whose it is, and what it may do
  ```

  Adding `--github comment` or `--github read` keeps the agents of its
  sessions from pushing. `--access full` (Everything) in place of `run`
  also lets the agent change the project's settings, secrets, members and
  schedules, as far as the person's role there allows, but takes no
  `--github`: it could change the settings that say what the project's
  agents may do.

What the person asks is a hint. "I want to use the fctl.sh CLI to manage
the xyz123 project" names one project, so they most likely want a token for
xyz123 alone: not a full sign-in, nor a token for every project of the
workspace xyz123 is in (`-w`). Suggest a token that runs sessions in that
project (`--access run -p xyz123`), or one that does everything there
(`--access full -p xyz123`) if they also want the agent to change its
settings. When they name more projects, or a workspace, the token takes
them all (`-p` repeated, `-w`); a full sign-in is for when they want the
agent to reach their account, or their workspaces' people, settings and
runners. Either way, the sessions the agent starts run as the person, on
their Claude subscription.

## Commands that return, and commands that follow

Most commands do one thing and return. Three keep running:

| Command | Returns | Keeps running |
|---|---|---|
| `fctl run … -d` | once the session is created | |
| `fctl run …` without `-d` | | following the session, also after it stops, until killed (Ctrl-C) |
| `fctl attach SESSION` | | until Ctrl-C, Ctrl-D or `/detach` in a terminal |
| `fctl attach --read-only SESSION`, `fctl logs -f SESSION` | at once, if the session had already stopped or failed when it connected | otherwise, also after the session stops |
| `fctl logs SESSION`, `fctl show`, `fctl sessions`, `fctl send`, `fctl resume`, `fctl stop`, `fctl interrupt` | once done | |

So in scripts and tool calls:

- Always pass `-d` (`--detach`) to `fctl run`, then wait as below.
- Use `fctl logs` without `-f`. To stream for a while, bound it:
  `timeout 600 fctl logs -f SESSION --json`.
- While a viewer is connected (`fctl attach`, `fctl logs -f`, the web app
  with its tab visible), its person counts as watching the session and
  gets no notifications about it.

Other things that wait for input:

- **stdin.** `fctl run` and `fctl send` without message text, and `fctl
  schedules add` without `-m` or `--task-file`, read stdin to its end when
  it isn't a terminal. Where stdin stays open, as in some agent harnesses,
  they hang. Pass the text as an argument, or `-f FILE` to `fctl run`, or
  pipe it in yourself (`printf '%s' "$text" | fctl send SESSION`).
- **Confirmations.** Deleting asks you to type the name in a terminal;
  without one, pass it: `fctl projects delete my-app --confirm my-app`
  (also `fctl workspaces delete --confirm NAME`, `fctl account delete
  --confirm EMAIL`, `fctl admin delete-user --confirm EMAIL`, `fctl admin
  delete-workspace --confirm NAME`). `fctl host uninstall` with sessions
  running needs `--wait` or `--force` without a terminal.
- **Confirming it's you.** Where the instance asks for it, sensitive actions
  (making access tokens, adding or registering runners, inviting admins,
  exporting data, deleting accounts, projects and workspaces) fail with
  `verification_required` until the token confirmed within the last 15
  minutes: `fctl account verify --code 123456` with a code from the
  person's authenticator app. Limited tokens can't confirm, and of these
  actions reach only deleting a project, which they leave to a person.

## Output and exit codes

- With `--json`, a command prints one JSON document on stdout, in the
  shapes of [the HTTP API](api.md): `fctl run -d --json` and `fctl show
  --json` print a session (`.id`, `.number`, `.status`, `.status_reason`,
  `.branch`, `.pull_requests`), `fctl sessions --json` prints `{"sessions":
  […], "next_before": …}`.
- `fctl logs --json` prints one event per line (JSON Lines), with or
  without `-f`: read it with `jq -s` as an array, or line by line.
- Errors go to stderr as text, `error: …`, with or without `--json`.
- Exit codes: 0 for success, 1 for an error (the API refused, nothing to
  do, a network failure), 2 for a usage error (an unknown flag, a missing
  argument). `--help` exits with 0.
- Optional fields are left out when empty: `.pull_requests` is absent, not
  `[]`, until the session has one (`jq '.pull_requests // []'`).

## Naming things

- A session is its id (`ses_01J…`), its URL (`https://app.fctl.sh/s/ses_…`)
  or `project#number` (`my-app#12`). `project#number` looks among the
  project's latest 200 sessions: scripts keep the id.
- Projects and workspaces take their slug or id. Commands that take a
  project use `-p`, else the one `fctl projects use` chose; commands that
  take a workspace use `-w`, else `fctl workspaces use`, else your only
  one. So `fctl sessions` without `-p` lists only the default project's
  sessions when one is set; `-w` lists a workspace's.
- Schedules take their id (`sch_…`) or name, memories `mem_…`,
  retrospectives `rtr_…`, tokens their name or `tok_…`.

## Waiting for a session

A session's status says where it is:

| Status | What it means | What to do |
|---|---|---|
| `queued` | waiting for a runner; `status_reason` says why | wait a little; if no runner is online, or none allows its image or size, tell a person |
| `provisioning`, `preparing` | its sandbox is being made; the setup script runs | wait |
| `running` | a turn is under way | wait |
| `idle` | the turn ended: it waits for a message, or for its background tasks (`status_reason` "Waiting for 2 background tasks"), which start a turn of their own when they end | read the reply; message it, or leave it |
| `paused` | its person's plan hit a usage limit; it continues by itself at `resume_at` | wait, or tell a person |
| `stopping`, `stopped` | its sandbox stopped (idle timeout, `fctl stop`, its runner went away); a message resumes it | message it to carry on |
| `failed` | `status_reason` says why | `fctl resume` restarts it, on your subscription |

Every turn ends with a `result` event from Claude in the transcript, which
holds the agent's last message. Counting them tells a turn that ended from
one that hasn't started yet, which the status alone doesn't right after a
message:

```sh
turns() { fctl logs "$1" --json | jq -s '[.[] | select(.kind == "claude" and .data.type == "result")] | length'; }
reply() { fctl logs "$1" --json | jq -s -r '[.[] | select(.kind == "claude" and .data.type == "result")] | last | .data.result'; }
wait_turn() {   # until the session has more than $2 finished turns, or can't go on
  while [ "$(turns "$1")" -le "$2" ]; do
    case $(fctl show "$1" --json | jq -r .status) in failed|paused) return 1 ;; esac
    sleep 20
  done
}

id=$(fctl run -p my-app -d --json "Fix the flaky login test and open a pull request" | jq -r .id)
wait_turn "$id" 0 && reply "$id"

n=$(turns "$id")
fctl send "$id" "Also add a regression test"
wait_turn "$id" "$n" && reply "$id"
```

- Turns take minutes to hours. Poll every 20 to 60 seconds, not faster:
  each `fctl logs` reads the whole transcript (the API reads it from an
  event on: [Events](api.md#events)).
- `fctl show SESSION --json` has the branch, the pull requests with their
  state and checks (`.pull_requests[] | {url, state, checks}`), the model,
  and counters of turns, tool calls and changed files (`.stats`).
- A session that waits for a message stops after its project's idle timeout
  (ten minutes unless the project says otherwise); a message resumes it, in
  the same sandbox and conversation while its runner still has them.

## Reading a transcript

Each line of `fctl logs --json` is an event: `{"seq": 21, "ts": …, "kind":
"…", "data": {…}}`, with `seq` counting up from 1 and `ts` in milliseconds
since the epoch. The kinds:

| `kind` | `data` |
|---|---|
| `user_message` | what a person, fctl or a tool sent: `text`, `author`, `source` (`web`, `cli`, `api`, `remote_control`, `system`), files, and what brought it (a follow-up, a review, a ticket) |
| `claude` | a message of Claude Code's stream-json output, as Claude Code wrote it: `type` is `system`, `assistant` (text and tool calls in `.message.content`), `user` (tool results), `result` (the end of a turn: `.result` is the final text, `.is_error`), `rate_limit_event` |
| `status` | `status`, `reason`, `resume_at` |
| `git` | the repositories' state: `branch`, `base`, `head`, `ahead`, `behind`, `files`, `uncommitted`, `pushed`, `last_commit`, `pull_requests`, and the other repositories in `repos` |
| `log` | output of provisioning, the setup script and the supervisor |
| `notice` | something to notice: a usage-limit pause, a runner that went away, a note added to memory, a retrospective, a follow-up held back |
| `remote_control`, `interrupt` | Remote Control became available or not; someone interrupted a turn |

Some useful filters:

```sh
fctl logs "$id" --json | jq -r 'select(.kind == "claude" and .data.type == "assistant") | .data.message.content[] | select(.type == "text") | .text'
fctl logs "$id" --json | jq -c 'select(.kind == "claude" and .data.type == "assistant") | .data.message.content[] | select(.type == "tool_use") | {name, input}'
fctl logs "$id" --json | jq -s 'map(select(.kind == "git")) | last | .data'
```

Images come without their bytes. The session keeps a copy of each image
its agent's tools returned (a `Read` of a screenshot) for 7 days, which
`kept` in the image's `source` describes and the API returns
([Events](api.md#events)):

```sh
fctl logs "$id" --json | jq -r 'select(.kind == "claude" and .data.type == "user") | .data.message.content | arrays | .[]
  | select(.type == "tool_result") | .content | arrays | .[] | .source.kept.id // empty'
curl -sS -H "Authorization: Bearer $FCTL_TOKEN" -o after.png "$FCTL_SERVER/api/v1/sessions/$id/images/img_01J…"
```

`fctl logs SESSION` without `--json` prints the transcript as people read
it, and `fctl search WORDS --json` finds sessions by what people and agents
wrote.

## Steering a session

- `fctl send SESSION "…"` reaches the agent at its next step, with `-a FILE`
  for files (up to 10, of 20 MB each); a stopped session resumes for it, on
  the Claude subscription it ran on. Each call is a new message: through
  the API, a `client_id` makes a retried message arrive once.
- `fctl interrupt SESSION` stops the current turn; the session waits for a
  message.
- `fctl stop SESSION` ends the session and keeps its sandbox; `fctl resume
  SESSION "…"` starts it again on your subscription, a failed one too,
  which a message doesn't.
- A token that may comment or read on GitHub can only read a session whose
  agent started able to do more, so it can't steer an agent that pushes.
- Review agents' sessions take no messages.
- People can share a session with others in its workspace, and anyone who
  may message it can see, and have its agent hand over, what is in its
  sandbox: the project's secrets, its GitHub tokens and the Claude token of
  the person it runs for.

## Things to avoid

- `fctl run` without `-d`, `fctl attach` and `fctl logs -f` in tool calls
  and scripts: they don't return when the session ends.
- Commands that read stdin (`fctl run`, `fctl send` and `fctl schedules
  add` without their text) where stdin stays open.
- Starting sessions in a loop. Each session spends its person's Claude
  plan and takes a runner's slot (four each by default); the ones runners
  can't take queue. Fan out to a handful at a time and wait for them.
- Sending a message again because the first seemed lost: read the
  transcript first, or send it through the API with a `client_id`.
- Passing text you didn't write as your instruction. What `fctl run` and
  `fctl send` carry reaches the agent as the token's person's words. Text
  from issues, emails, chats and webhooks goes in a when-asked schedule's
  `context` ([below](#from-any-tool)), which the agent gets marked as
  information, not instructions.
- More GitHub access than the task needs. Reviews, triage and planning run
  with `--github comment` or `--github read`; a project's maintainers can
  cap every session (`fctl projects set --github comment`).
- `--env` for credentials: its values are not secret. Secrets go in the
  project's secrets, with the value on stdin (`printenv STRIPE_KEY | fctl
  projects secrets set STRIPE_KEY -p my-app`), never as an argument;
  anything the agent runs can read both.
- Pushing to a session's branch from outside while its agent works, force
  pushing it, or reusing it once its pull request merged: fctl pushes it
  after every turn until the merge, then no more. Message the agent, or
  start a new session.
- Deleting projects, workspaces or accounts, or uninstalling and draining
  hosts, unless a person asked for exactly that.
- Polling faster than every 20 seconds, or reading whole transcripts in a
  tight loop.

## Recipes

### Hand work to fctl and collect the result

[Waiting for a session](#waiting-for-a-session) has the script. Variations:

```sh
fctl run -p my-app -d --json --github read "Explain how sessions are scheduled, with file references"
fctl run -p my-app -d --json -a screenshot.png "Make the header match this"
fctl run -p my-app -d --json -f task.md --title "Move to Postgres" --branch db/postgres --base develop
fctl run -p my-app -d --json --model opus --effort high "…"
```

The pull request is in `.pull_requests` once the agent opened one; its
URL, state and checks come with it.

### Several tasks at once

Each session has its own sandbox, branch and pull request, so independent
tasks run side by side:

```sh
while IFS= read -r task; do
  fctl run -p my-app -d --json "$task" | jq -r .id
done < tasks.txt > sessions.txt
fctl sessions -p my-app --active --json | jq -r '.sessions[] | "\(.number) \(.status) \(.title)"'
```

Runners take as many sessions as their capacity allows, and the rest wait
in the queue. Tasks that touch the same files make pull requests that
conflict; with conflict follow-ups on, each agent merges the base branch in
once the other merges.

### From any tool

Any program that can make an HTTP request starts agents through a schedule
that runs when asked: the schedule holds the task, and the program sends
what it knows as `context`.

1. A person sets up the schedule and a token:

   ```sh
   fctl schedules add "Triage an issue" -p my-app --when-asked --github comment \
     -m "Triage the issue that comes with this: label it, say what is missing to act on it, link duplicates and point at the code it concerns."
   fctl token "Issue triage" --access run -p my-app --github comment --days 90
   ```

2. The program posts to the schedule's runs, with at most 20,000
   characters of context:

   ```sh
   curl -sS -H "Authorization: Bearer $FCTL_TOKEN" -H "Content-Type: application/json" \
     -d '{"context": "Issue #41: The login page hangs on Safari\n\nSteps: …"}' \
     "$FCTL_SERVER/api/v1/schedules/sch_…/runs"
   ```

   The answer is the run: `.outcome` (`started`, `skipped` or `failed`),
   `.reason`, and `.session.id` of the session it started. From a shell:
   `printf '%s' "$text" | fctl schedules run sch_… --context-file - --json`.

The agent gets the task, a note that a tool started it and nobody watches
it start, and the context inside `<context>`, marked as information from
outside fctl, not instructions. Tools may run one schedule 20 times an
hour; pausing the schedule (`fctl schedules pause sch_…`) stops them all.
A tool that can't add a header to its webhooks needs a small relay that
adds it and picks out the fields: a serverless function, or a no-code tool
such as Zapier, Make or n8n. [Automation](automation.md#start-agents-from-other-tools)
has more.

### From GitHub Actions

[Automation](automation.md#review-pull-requests-from-github-actions) has a
workflow that reviews each pull request with a token in the repository's
secrets, keeping one session per pull request, found by its branch. The
same shape triages new issues:

```yaml
on:
  issues:
    types: [opened]
jobs:
  triage:
    runs-on: ubuntu-latest
    steps:
      - run: |
          curl -fsSL https://fctl.sh/install | FCTL_NO_COMPLETIONS=1 sh
          echo "$HOME/.local/bin" >> "$GITHUB_PATH"
      - env:
          FCTL_SERVER: https://app.fctl.sh
          FCTL_TOKEN: ${{ secrets.FCTL_TOKEN }}
          ISSUE: ${{ github.event.issue.number }}
        run: |
          fctl run -p my-app --github comment -d "Triage issue #$ISSUE: label it, say what is missing to act on it, and point at the code it concerns."
```

Pass what comes from GitHub (titles, bodies) through `env`, never `${{ }}`
inside the script, where it would run as shell. Issue bodies come from
anyone: send them as a when-asked schedule's context instead of in the
task.

### On a schedule

```sh
fctl schedules add "Weekly dependencies" -p my-app --cron "0 9 * * 1" --tz Europe/Stockholm \
  -m "Update the dependencies that have new patch or minor releases, run the tests and open a pull request."
fctl schedules add "Release notes" -p my-app --at 2026-10-30T16:00+01:00 -m "Draft the release notes."
fctl schedules -p my-app --json          # each with .next_run_at and .last_run
fctl schedules runs "Weekly dependencies" -p my-app
```

- `--cron` takes five fields or `@hourly`, `@daily`, `@weekly`, `@monthly`
  and `@yearly`, at most every 15 minutes; `--tz` is the machine's time
  zone unless given, UTC in most CI. `--at` takes `2h`, `30m`, `1d` or a
  time with its UTC offset. `--paused` sets it up without running it.
- Runs go on the subscription of the schedule's owner, who set it up; a
  change by someone else moves it to theirs. A run is skipped while the
  last one still works, or while the owner's plan is at its limit.
- Setting up or changing a schedule from a token takes `--access full`,
  since its sessions start after the token is gone; a `run` token runs
  existing ones.

### Linear

1. A workspace admin who administers the Linear workspace connects it: `fctl
   workspaces linear connect` (opens Linear, where they install fctl's app
   and choose the teams it sees).
2. A project maintainer chooses the team: `fctl workspaces linear teams`,
   then `fctl projects set -p my-app --tickets linear --linear-team ENG`
   (`--linear-project "Mobile app"` for one Linear project of it).
3. Each person links their Linear account once: `fctl account linear
   connect`.

Then assigning an issue to fctl in Linear (which makes fctl its delegate)
or mentioning fctl in a comment starts a session of the project as that
person, on their subscription. The issue moves to the team's first started
state, the branch carries its key (`fctl/12-eng-123-fix-the-login-loop`),
and the agent's steps, pull requests and replies show in the issue's agent
session, where replies reach the agent. With the `explore` label the agent
replies with a plan and changes nothing until someone asks. In the
sandbox, the agent reads and writes the team's issues with `fctl ticket`.

### Review agents and follow-ups

```sh
fctl review settings -p my-app --on --model opus --effort high   # a project owner: on their subscription
fctl review settings -p my-app --instructions-file review.md
fctl review settings -p my-app --bots dependabot,renovate         # apps whose pull requests start reviews
fctl review start -p my-app 12                                    # review a pull request's head now
fctl review -p my-app --json                                      # reviews and what came of each
fctl projects set -p my-app --follow-ups all                      # comments, checks, conflicts, closed
```

Each new commit of a pull request in the project's writable repositories
then gets a review; when an fctl agent opened the pull request, its session
gets the findings, fixes what is real and answers in the threads. Sessions
that list the pull request show how the review stands beside its checks,
out of their count (`checks.review` in `fctl show --json`): waiting for a
runner, reviewing, or how many issues it found. Follow-ups
bring that agent back for `@fctl` comments from people who can write to the
repository, failed checks, conflicts with the base branch and merges. Both
need the instance's GitHub webhook ([Self-hosting](self-hosting.md#follow-ups)).

### Set up a project

What a workspace admin, or a project's maintainer, can do from the CLI:

```sh
fctl projects create "My app" --repo my-org/my-app -w acme --slug my-app
fctl projects repos add my-org/my-lib --write -p my-app
fctl projects set -p my-app --setup-script @setup.sh --env NODE_ENV=test --idle-timeout 30
fctl projects set -p my-app --image ghcr.io/acme/agent:1      # or --build .devcontainer/devcontainer.json
fctl projects set -p my-app --cpus 8 --memory 16g --package-caches on
fctl projects set -p my-app --github push --pr-descriptions balanced --follow-ups all
fctl projects set -p my-app --pr-instructions-file prs.md   # how they handle pull requests and issues
fctl projects set -p my-app --tickets github
fctl projects set -p my-app --model opus --effort high --instructions-file agents.md
fctl projects set -p my-app --auto-pull-request on --remote-control on --connectors off
printenv STRIPE_KEY | fctl projects secrets set STRIPE_KEY -p my-app
fctl projects members add grace@example.com --role maintainer -p my-app
fctl projects show my-app --json
```

And for a session: `fctl share SESSION grace@example.com --role viewer`
lets someone else in its workspace read it (`editor` also messages its
agent), `fctl unshare SESSION grace@example.com` takes it back, and `fctl
archive SESSION` takes it out of the lists.

### Bring a machine online

On the machine, by its owner: install a driver (Apple's `container` on
macOS 26 with Apple silicon, then `container system start`; Lima; or
Docker), then:

```sh
curl -fsSL https://fctl.sh/install | sh
fctl login app.fctl.sh
fctl host install --workspace acme          # or --token fctl_rt_… from the workspace's Runners tab
fctl host doctor && fctl host status
```

`fctl host drain --wait` empties it for maintenance and `fctl host undrain`
takes sessions again. [Getting started](getting-started.md#5-bring-a-machine-online)
and [Hosts](cli.md#hosts) have the rest.

### Reports

A token with `--access read` reads and starts nothing:

```sh
fctl sessions --active --json
fctl search "flaky test" --json
fctl retro list -p my-app --outcome blocked --json
fctl memory -p my-app --json
fctl review -p my-app --json
```

### Teach your agent to use fctl

To let a coding agent hand work to fctl, give it `FCTL_SERVER` and a
limited `FCTL_TOKEN` in its environment and a few lines in its
instructions (`CLAUDE.md`, `AGENTS.md`):

```markdown
## fctl
Hand longer tasks to fctl (https://fctl.sh/llms.txt) instead of doing them here:
`fctl run -p my-app -d --json "<task>"` starts one; wait for its turn by counting
`result` events in `fctl logs <id> --json`, and read the pull request in `fctl show <id> --json`.
Never run `fctl run` without `-d`, `fctl attach` or `fctl logs -f`: they don't return.
```

## Inside a session's sandbox

An agent that fctl runs finds what it needs in its system prompt and in
`~/AGENTS.md`, which `~/.claude/CLAUDE.md` imports. Its sandbox has no fctl
credentials: these commands reach the control plane through fctl-agent,
for its own session and project only.

```sh
fctl edit --title "Fix the flaky login test" --description "Retries the redirect and tests it."
fctl memory                                  # the project's notes, now
fctl memory add "The e2e tests need Chromium: run npx playwright install chromium first."
fctl memory stale mem_… --reason "The script moved to xtask"
fctl retro list                              # this session's retrospectives, with their ids
fctl retro --outcome completed --worked-on "…" --approach "…" --challenges "…" --insights "…"
fctl retro update rtr_… --outcome completed --challenges "…"
fctl ticket list --search login              # where the project's tracker is Linear or one repository's issues
fctl ticket comment ENG-123 "…"
fctl publish after.png                       # where the project publishes screenshots
fctl resolve PRRT_…                          # agents that comment: resolve a review thread
fctl review threads && fctl review submit review.json   # review agents
```

- The repository is at `~/workspace`, on the session's branch; other
  repositories go in `~/repos/<owner>/<name>`; attached files are in
  `~/attachments/`. `FCTL_SESSION_ID`, `FCTL_SESSION_URL`,
  `FCTL_SANDBOX_CPUS` and `FCTL_SANDBOX_MEMORY_MB` say where it is.
- git and `gh` are signed in with tokens for the project's repositories, as
  far as the session's GitHub access allows, refreshed each hour.
- fctl pushes the branch after every turn until its pull request merges;
  agents commit as they go and never force-push.
- Notes are at most 2,000 characters, a project keeps 500, and a note
  replaces at most 20 others. A session keeps at most 50 retrospectives and
  publishes at most 100 screenshots of 5 MB. In an hour, an agent comments
  on tickets at most 30 times, files at most 20, changes at most 60 and
  reads its tracker at most 300 times.
- Agents in a session can't start other sessions: they hold no fctl
  credentials. A project secret with an access token would let them, on
  that token's person's subscription; treat that as giving every agent of
  the project that person's access.
