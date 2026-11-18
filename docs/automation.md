# Automation

Agents start when you ask, at the times you set, when something happens
on GitHub (an issue opened, a pull request merged, checks failing on
main), when someone delegates a ticket to fctl in Linear, and when your
CI, a bot or another tool asks: schedules start sessions on their own,
and an access token lets a workflow start them, limited to what it is
for. And fctl tells your other tools what agents do as it happens, with
webhooks. This page shows how to set up schedules, those that run on
events among them, how to delegate Linear tickets, how to start agents
from other tools, such as another issue tracker, how to start them for
your people from a portal of your own with a service account, how to
hear of what agents do with webhooks, how to make a token, how to run
review agents on your pull requests from GitHub Actions, and other uses:
triaging issues, nightly maintenance, planning, dashboards.

## Schedules

A schedule is a task that fctl starts a session for on its own: once at a
set time, or again and again, every hour, day, weekday, week or month, or
on any cron expression, in a time zone you choose; each time something
happens on GitHub ([below](#run-on-events)); or only when asked, by you
or another tool ([below](#start-agents-from-other-tools)). Set one up
under the project's **Schedules** tab, which offers ideas to start from
(triaging each new issue, working on issues you label, learning from
merged pull requests, fixing failing checks on main, trying each release,
dependency updates, tidying the project's memory, learning from the
week's reviews), or with `fctl schedules add`;
**Schedule**, beside Start in the project's task box, and in the Agent
Floor's in a wide window, turns what you wrote there into one.

- **Each run is a session.** It is named after the schedule, works on a
  branch of its own and shows in the project's sessions with the
  schedule's mark; its page links to the schedule, and the schedule lists
  what each of its runs started. The agent gets the task, then a note
  from fctl: which schedule started it and when, its previous run, and
  that nobody watches it start, so that it works without waiting for
  answers and says plainly what only a person can decide.
- **Out of the lists, when it runs often.** Turn on **Keep its sessions
  off session lists** for a schedule such as an hourly triage (`fctl
  schedules add --hide-sessions`, or `edit --hide-sessions`): its
  sessions, those it started already too, stay out of Working now, the
  sidebar, the project's sessions and `fctl sessions` (`--hidden` keeps
  them in). Its runs list them, and search finds them.
- **On whose subscription.** A schedule runs on the Claude subscription of
  the person who set it up, its owner, who follows its sessions as they
  follow the ones they start (unless they turned that off in Settings). A
  change by someone else, as one of the project's maintainers or owners,
  runs it on theirs from then on: nobody puts work on someone else's
  subscription. Running one now, from the tab or with `fctl schedules
  run`, starts a session on your own.
- **Runs don't pile up.** A run is skipped while the schedule's last
  session still works on its task (for one that runs on events, the last
  on the same issue, pull request or branch), and while its owner's plan
  is at its limit, until the limit resets; a schedule that runs once
  waits for the reset instead. The schedule says so, and why, in its
  runs.
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

## Run on events

A schedule can wait for something to happen on GitHub in the project's
repositories, rather than for a time: choose **On an event** in its form,
or `fctl schedules add --on`. Each time it happens, fctl starts a run of
its own, whose agent gets the task, then what GitHub said of it:

| Event | `--on` | What the agent gets | For |
|---|---|---|---|
| **An issue is opened** | `issue-opened` | the issue: title, text, labels, who opened it | triaging each issue as it comes in |
| **An issue gets a label** | `issue-labeled` | the issue, and who added the label | handing an issue to an agent: label it `fctl` |
| **A pull request is opened** | `pr-opened` | the pull request, its branches; a draft once it's marked ready | checks a review agent doesn't make |
| **A pull request merges** | `pr-merged` | the pull request, who merged it and as which commit | learning from what people said in its review |
| **A release is published** | `release` | the release: tag, name, notes | trying or announcing each release |
| **Checks fail on a branch** | `checks-failed` | the commit, and the checks that failed once all ended | fixing the default branch when it breaks |

- **Narrow it down.** In the project's repositories its agents write to,
  or the ones you choose (`--repo`, a read-only one too: a dependency's
  releases, say); with one of some labels (`--label`: a bug report's
  template label, say); into some branches (`--branch`), or for failing
  checks the branches it watches (the default branch unless you choose);
  opened by anyone, by people who may write to the repository and fctl's
  agents, or by fctl's agents alone (`--by anyone|writers|agents`).
  Pull requests opened start runs only for people who may write to the
  repository unless you choose anyone, as review agents do: an outsider's
  pull request starts nothing on your subscription by itself.
- **What GitHub says is information.** Anyone can open an issue on a
  public repository, so the agent gets what they wrote after fctl's note,
  marked as information for the task and never as instructions. A
  schedule that anyone on GitHub can start, on issues opened or on pull
  requests opened by anyone, runs at **Triage only** unless you choose
  more (`--github`): its agents label and comment through fctl and write
  nothing else, so that what an outsider wrote can't steer an agent that
  pushes. Choose more only where you trust whoever can open one, as on a
  private repository, and the form says so as you do.
- **One run for each event.** GitHub sends a delivery again when it got
  no answer in time, and people redeliver one; fctl starts a run for each
  issue opened, pull request opened or merged, release and failed commit
  once. A label put back on after it was taken off runs it again. While a
  run still works on an issue, a pull request or a branch's checks, events
  about the same one start none, so an issue that gets two of its labels
  at once, or a branch whose next commit fails too, starts one run.
- **At most 3 an hour and 10 a day, unless you say otherwise.** Each
  run is a session on its owner's plan, and anyone can open issues on a
  public repository, so a schedule's events start at most so many
  sessions in the last hour and the last 24 hours: past either, they
  start none until there's room again, and the schedule's runs say so,
  with until when. Set how many in the form's **At most … sessions an
  hour and … a day** (up to 100 an hour and 1,000 a day), or with `fctl
  schedules add` and `edit`'s `--max-per-hour` and `--max-per-day`. Turn
  on **Keep its sessions off session lists** for one that runs often.
- **The GitHub App hears of it.** Events come through the instance's
  GitHub App and its webhook, subscribed to **Issues** for issues,
  **Pull request**, **Release** and **Check suite**
  ([Self-hosting](self-hosting.md#1-create-the-app)). The form and `fctl
  schedules` say which events fctl wouldn't hear of, and why, before you
  choose one.

```sh
fctl schedules add "Triage new issues" -p web --on issue-opened --github triage --hide-sessions \
  --max-per-hour 10 --max-per-day 40 \
  -m "Triage the issue: label it, ask for what is missing to act on it, and point at the code it concerns."
fctl schedules add "Learn from merges" -p web --on pr-merged --by agents --hide-sessions \
  -m "Read what people said in the pull request's review, and propose what agents should know in AGENTS.md."
fctl schedules runs "Triage new issues" -p web    # each run, with the issue that started it
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
   --github triage -m "Triage the issue that comes with this: label it,
   say what is missing to act on it, link duplicates and point at the code
   it concerns."` Its details show the request below, with its id, ready
   to copy (any schedule can be run this way).
2. **A token that runs sessions.** `fctl token "Issue triage" --access run
   -p web --github triage --days 90`.
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

## Service accounts

A portal of your own, where your people pick a project and describe a
task, a chat bot, or a form behind a script starts agents for them with
one of the workspace's service accounts. Each request names the person
it's for, and the session is theirs: on their Claude subscription, within
what they may do in the project, in their lists in fctl, and they hear
when it finishes or asks something.

1. **Make one** (the workspace's admins): the workspace's **Service
   accounts** tab, or `fctl service-accounts create "Acme portal"
   --description "Our agent portal at portal.acme.dev"`. Its secret
   (`fctl_svc_…`) shows once: give it to your program, and nobody else.
   What it says it is, people read as fctl asks them.
2. **People allow it.** It starts sessions only for those who allowed it:
   fctl asks each member on their Agent Floor, and each one who joins
   later, and they change their answer in the workspace's Service
   accounts tab or with `fctl service-accounts allow "Acme portal"` (or
   `deny`). Where the instance's admins turned asking off, as an
   organization that runs fctl for its own people may, it acts for anyone
   in the workspace. `fctl service-accounts people "Acme portal"` lists
   who allowed it.
3. **Your program names the person** by their address (or fctl id) in
   the `Fctl-On-Behalf-Of` header:

   ```sh
   curl -X POST https://app.fctl.sh/api/v1/projects/web/sessions \
     -H "Authorization: Bearer $FCTL_TOKEN" \
     -H "Fctl-On-Behalf-Of: ada@acme.dev" \
     -H "Content-Type: application/json" \
     -d '{"prompt": "Fix the login redirect"}'
   ```

   Or with the CLI, which takes the token as `FCTL_TOKEN` too:
   `fctl run -p web --on-behalf-of ada@acme.dev -d "Fix the login
   redirect"`. Someone it can't act for (not a member, hasn't allowed
   it, disabled, two-factor sign-in not set up) gets a refusal whose
   `field` is `on_behalf_of`; someone without the role it needs there,
   such as editor to start sessions, gets `forbidden`, and `not_found`
   for a project they can't see. Its `message` says why, to show them.

What else it does: it messages, interrupts and stops the sessions of the
person it acts for (`fctl send … --on-behalf-of`), runs schedules
(`fctl schedules run "Nightly triage" -p web`, for the person the schedule
runs for, or the one it names), and reads the workspace's projects,
sessions and schedules (`fctl sessions --mine` lists those it started,
`fctl logs web#12` what an agent did). It changes nothing else: no
settings, people, secrets or tokens. To hear when its sessions finish or
ask something, add a webhook narrowed to them: `fctl webhooks add
https://portal.acme.dev/fctl --events session.finished,session.question
--started-by service-accounts`.

Every session it starts says so, on its page, in `fctl show`, in the
audit log and in what webhooks post. Its secret works while the admin who
made it is an admin of the workspace: when they leave, it stops until
another admin makes it a new one (`fctl service-accounts secret "Acme
portal"`), which you give your program. People's answers stay.

## Webhooks

A webhook posts what happens in a workspace to an address of yours as it
happens, signed: a session started, finished a turn, asked something or
has its pull requests ready for review; an agent's pull request opened or
merged; a review agent's review posted; a schedule's run that started
nothing. Slack hears when an agent needs someone, a deploy starts when a
schedule's session finishes, a dashboard follows sessions and reviews.

1. **Add one.** A workspace's admins add webhooks for all its projects,
   or some, on its **Webhooks** tab; a project's owners and maintainers
   add its own under its **Settings → Webhooks**. Or with the CLI:

   ```sh
   fctl webhooks add https://hooks.example.com/fctl -p web \
     --events session.question,session.ready_for_review,session.failed
   ```

   fctl first [checks the address](#checking-the-address): your receiver
   answers its challenge. Then it shows the webhook's signing secret
   (`whsec_…`) once: keep it where your receiver reads it. Those who manage the webhook read it again on
   its page (`fctl webhooks secret NAME`), and rotate it, which take
   confirming it's you where the instance asks for that.
2. **Choose what it hears of**: every event or some ([Webhook
   events](#webhook-events)), for a workspace's webhook its projects, and
   for sessions' events whose sessions: those people and tools start,
   schedules', an autopilot's, or those a Linear ticket started
   (`--started-by schedules`).
3. **Answer within 10 seconds** with a 2xx status, and do slow work after
   answering; check that each request is fctl's before acting on it
   (below).
4. **Send a test** of any event (**Send a test** on its page, `fctl
   webhooks test NAME --event session.ready_for_review`): a made-up event
   marked `test`, whose answer shows at once.

Addresses are `https://` on the public internet: fctl doesn't post to
private networks, loopback or local names. A self-hosted instance can
let webhooks reach hosts on its own network and post over `http://`
([Self-hosting](self-hosting.md#webhooks-to-your-own-network)); app.fctl.sh
doesn't. A development instance takes `http://` and local addresses, to
try a receiver on the same machine. The address field says what the
instance takes, and so does `fctl webhooks events`. A change to a webhook
reaches new events within half a minute.

### Checking the address

As a webhook is added, and as it's pointed elsewhere, fctl posts the
address a challenge, as Slack does before its events go to an app, and
saves the address only once the answer gives the challenge back. It
shows that the address is yours and takes fctl's requests before any
event goes there. The request is signed like the others, with
`fctl-event: url_verification`:

```json
{
  "id": "evt_01KC9R2W…",
  "type": "url_verification",
  "timestamp": "2026-11-17T14:01:52.113Z",
  "challenge": "3eZbrw1aBm2rZgRNFdxV2595E9CY3gmdALWMmHkvFXO"
}
```

Answer it with a 2xx status and the challenge, as text or as
`{"challenge": "…"}` (or a form's `challenge=…`), within 10 seconds, and
before checking its signature: a new webhook's secret reaches you only
once fctl has added it. The challenge asks for nothing else, and nothing
happened. An answer without it says why under the address (`fctl webhooks
add` and `edit --url` say so too), and the webhook stays as it was. A
workspace's webhooks post at most 20 challenges an hour.

### What a request carries

Each is a `POST` of JSON, in the shape Standard Webhooks gives: the
event's id and type, when it happened, the workspace and the project it
happened in, and its data, here a session's pull requests that wait for
a person's review:

```json
{
  "id": "evt_01KC9R3M…",
  "type": "session.ready_for_review",
  "timestamp": "2026-11-17T14:02:11.408Z",
  "workspace": { "id": "wsp_01KB…", "slug": "acme", "name": "Acme" },
  "project": { "id": "prj_01KB…", "slug": "web", "name": "Web" },
  "data": {
    "session": {
      "id": "ses_01KC…",
      "ref": "web#12",
      "url": "https://app.fctl.sh/s/ses_01KC…",
      "title": "Retry failed uploads in the sync worker",
      "status": "idle",
      "started_by": "person",
      "created_by": { "id": "usr_01KA…", "name": "Ada" },
      "pull_requests": [{ "repo": "acme/web", "number": 45, "state": "open", "…": "…" }]
    },
    "ready": [
      {
        "repo": "acme/web",
        "number": 45,
        "url": "https://github.com/acme/web/pull/45",
        "title": "Retry failed uploads in the sync worker",
        "state": "open",
        "checks": { "passed": 7, "failed": 0, "pending": 0, "sha": "3f2a1c9…", "review": "passed" }
      }
    ]
  }
}
```

Sessions' events carry the session as `data.session` (its schedule,
autopilot, ticket or service account where it started from one, and
`started_by`, how it started); agents' pull requests'
`data.session` and `data.pull_request`; reviews' `data.review` (its
pull request, commit, state, and how many new findings it reported,
earlier ones it found fixed and those still open); schedules'
`data.schedule` with the run that started nothing, or why fctl paused it.
The API's types are `HookEnvelope`, `HookSessionData`,
`HookPullRequestData`, `HookReviewData` and `HookScheduleData`
([crates/fctl-proto/src/hooks.rs](https://github.com/hyperstructure/fctl/blob/main/crates/fctl-proto/src/hooks.rs)).

| Header | What it says |
|---|---|
| `webhook-id` | The event's id: the same in every attempt and resend, to skip one you had |
| `webhook-timestamp` | When fctl posted it, in seconds |
| `webhook-signature` | `v1,` and its signature; for a day after the secret was rotated, two, separated by a space |
| `fctl-event` | Its type, as in `type` |
| `fctl-workspace-id`, `fctl-project-id` | Where it happened |
| `fctl-delivery`, `fctl-webhook`, `fctl-attempt` | The delivery, the webhook, and which attempt this is |
| `fctl-test` | `1` on a test |

### Check that it's fctl's

fctl signs as [Standard Webhooks](https://www.standardwebhooks.com)
does: `v1,` and the HMAC-SHA256 of `{webhook-id}.{webhook-timestamp}.{body}`
in base64, keyed with the secret's bytes (the base64 after `whsec_`). Its
libraries check a request in one call:

```js
import { Webhook } from "standardwebhooks";

// `body` is the request's body as it came, not parsed JSON.
const sent = JSON.parse(body);
if (sent.type === "url_verification") return new Response(sent.challenge); // fctl checking the address
const event = new Webhook(process.env.FCTL_WEBHOOK_SECRET).verify(body, headers); // throws if it isn't fctl's
if (event.workspace.id !== "wsp_01KB…" || event.project?.id !== "prj_01KB…") throw new Error("Not ours");
```

Check the body byte for byte as it came, refuse a timestamp more than
five minutes from your clock (it may be a replay), and check the
workspace and project it names. The webhook's page shows the whole check
with its ids, in Node.js and Python, and `fctl webhooks verify` checks a
request as a receiver does, or one you saved earlier by its signature
alone (`--any-age`). **Rotate secret** makes a new one, with which fctl
signs from then on; the old one signs beside it for a day, so that your
receiver switches without missing a request.

### Webhook events

| Event | When |
|---|---|
| `session.started` | Someone, a schedule, an autopilot or a ticket started a session; it waits for a runner. |
| `session.finished` | Its agent's turn ended with no background work running, and it waits for a message: with the start of its reply (`data.message`). Sent once it has waited 5 seconds, as background work's own turn comes within them. |
| `session.ready_for_review` | Its agent finished, and its open pull requests (not drafts) passed every check, with no review agent at work on them, for two minutes: they wait for a person (`data.ready`). Once for each set of commits. It needs the instance's GitHub webhook, which tells fctl of checks ([Self-hosting](self-hosting.md#pull-request-state-and-checks)). |
| `session.question` | Its agent asked something with `fctl ask`, with the answers people can click. |
| `session.follow_up` | fctl brought its agent back to its pull request: a comment that addresses it, failed checks, conflicts, a merge or a close (`data.follow_up.kind` is `github`, with GitHub's `events`), or a review's findings (`review`). |
| `session.paused` | Its Claude plan reached a usage limit; it carries on once the limit resets. |
| `session.stopped` | Its sandbox stopped: it waited long enough, its work ran out of time, or someone stopped it. A message resumes it. |
| `session.failed` | It couldn't start or carry on (`data.session.status_reason` says why). |
| `session.status_changed` | Any change of its status, with the one before (`data.previous_status`). The busiest event. |
| `pull_request.opened` | A session's agent opened a pull request, or found the one of its branch. |
| `pull_request.merged`, `pull_request.closed` | A pull request a session's agent opened merged, or closed without merging. |
| `review.queued`, `review.started` | A review agent's review of a commit waits for a runner; a runner took it. |
| `review.finished` | The review is on the pull request: `state` is `passed` (nothing to point out) or `commented`, with `findings`, `fixed` and `still_open`. |
| `review.failed`, `review.cancelled` | It couldn't run or post; a newer commit took its place, or someone stopped it. |
| `schedule.run_skipped` | A schedule's run at its time or on an event started no session, and says why: its last run still at work, its owner's plan at its limit, its cap of sessions an hour or a day reached, or a session that couldn't start. |
| `schedule.paused` | fctl paused a schedule: its owner can no longer run it, or its project was archived. |

A review agent's own session sends no sessions' events: its review's say
how it goes. `fctl webhooks events` and `GET /api/v1/webhook-events` list
them with what they mean.

### When a request fails

A request counts when the receiver answers with a 2xx status within 10
seconds; fctl doesn't follow redirects. Otherwise it goes again after 10
seconds, then a minute, 5 minutes, 30, 2 hours, 6 and 12 (later where
the answer's `Retry-After` asks), 8 attempts over about 21 hours. A
webhook whose own attempts all failed for three days turns itself off
as a delivery fails for good, and says why; turn it on again once its address answers (a test or a
resend that fails doesn't count toward the three days, and turning it on
again or pointing it elsewhere starts them afresh). Its page and `fctl
webhooks deliveries NAME --failed` list what went out in the last 30 days
(its newest 1,000), each with the request as fctl posted it, every
attempt and the start of each answer. Resend one (five times each), or
all that failed since a time once your receiver is back: `fctl webhooks
resend NAME --failed --since 6h`. At most 120 requests a minute go to
one webhook, and at most 1,000 wait for it: events past that are
dropped, and its page counts them. Requests can come out of order: go by
`timestamp`, and a session's `updated_at`.

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
| What its sessions' agents may do on GitHub | Push, comment, triage or read (below), for a token that runs sessions | `--github push`, `comment`, `triage` or `read` |

A token with any limit reaches projects and sessions only: never your
account, your workspaces' people and settings, your Claude connections or
your other tokens, so it can't make itself a token without limits. Only a
token that runs sessions holds their agents to commenting, triage or
reading: one
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

To create people's accounts and put them in workspaces and projects from
your identity provider or a script, a self-hosted instance can let its
admins make admin tokens instead
([Provision people with admin tokens](self-hosting.md#provision-people-with-admin-tokens)).
To start sessions for the people of a workspace, rather than as yourself,
from a portal or a bot of your own, use a [service
account](#service-accounts).

## What agents may do on GitHub

Each session's agent pushes, comments, triages or only reads, in the
repositories its project may write to:

| | Its GitHub token | The agent |
|---|---|---|
| **Push** | writes contents, pull requests, issues and workflows | commits, pushes its branch and opens pull requests, as agents always did |
| **Comment** | writes pull requests and issues, reads contents | reviews, comments, labels and resolves review threads; can't push or merge |
| **Triage** | reads | labels the project's tickets and comments on them, through fctl; writes nothing else |
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

An agent that triages is for work whose input anyone may have written,
such as going through new issues: whatever an issue tells it, it can't
change code, open or review pull requests, or close, edit or file
tickets. Its token only reads, and it labels and comments with `fctl
ticket` in its sandbox, which fctl does for it in the project's ticket
tracker (GitHub's issues or Linear's) and holds to that: on GitHub it
writes only on issues, with labels the repository has. Its session's
transcript shows each label it put on or took off and each comment. A
project without a ticket tracker gives it nothing to triage, and it only
reads.

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

- **Triage new issues.** Start from the Schedules tab's **Triage new
  issues** idea, which runs on each issue as it's opened
  ([Run on events](#run-on-events)); for a tracker other than GitHub's,
  it goes through the issues opened since its last run each weekday. From
  a workflow on `issues: types: [opened]`, start a session that triages:
  `fctl run -p web --github triage --detach "Triage issue #$ISSUE: label
  it, say what is missing to act on it, and point at the code it
  concerns."` Either way its agent only labels and comments.
- **Nightly maintenance.** A schedule does it without a workflow
  ([Schedules](#schedules)): `fctl schedules add "Nightly dependencies"
  -p web --cron "0 3 * * *" -m "Update the dependencies that have patch
  releases and open a pull request."`
- **Plan before building.** Choose **Read only** in the task box, or `fctl
  run --github read`, and ask for a plan: the agent reads the code, issues
  and pull requests and answers in the session, writing nothing to
  GitHub. A project whose agents should only ever read, triage or
  comment says so in its settings.
- **Tell a chat when agents need someone.** A webhook on
  `session.question`, `session.ready_for_review` and `session.failed`
  ([Webhooks](#webhooks)) posts to a small function that writes the
  session's `ref`, `title` and `url` to Slack or Teams; `review.finished`
  adds how each review went.
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
