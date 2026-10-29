# The HTTP API

The web app and the CLI do everything through one HTTP API, and so can any
program: start sessions, message their agents, read their transcripts, run
schedules, keep a project's memory. This page describes how to call it and
the endpoints programs use most. [The CLI](cli.md) is the easiest way in
from a shell; [fctl for agents](agents.md) has the patterns that go with
either.

The request and response bodies are the Rust types in
[`crates/fctl-proto/src/api.rs`](../crates/fctl-proto/src/api.rs), with
sessions in [`session.rs`](../crates/fctl-proto/src/session.rs) and events
in [`event.rs`](../crates/fctl-proto/src/event.rs), and as TypeScript in
[`web/src/gen`](../web/src/gen). Their comments are the reference for every
field. The API grows by adding fields, which are optional: ignore the ones
you don't know.

## Calling it

```sh
export FCTL_SERVER=https://app.fctl.sh FCTL_TOKEN=fctl_pat_…
curl -sS -H "Authorization: Bearer $FCTL_TOKEN" "$FCTL_SERVER/api/v1/sessions?status=active"
```

- **Base URL**: the instance's address followed by `/api/v1`.
- **Authentication**: `Authorization: Bearer <token>`, with an access token
  (`fctl_pat_…`, [limited to what it is for](automation.md#access-tokens))
  or the CLI's own. The web app's cookie works only from the app itself.
- **Bodies** are JSON (`Content-Type: application/json`), except file
  uploads, which are `application/octet-stream`.
- **Ids** have a type prefix: `ses_` sessions, `prj_` projects, `wsp_`
  workspaces, `sch_` schedules, `srn_` schedule runs, `mem_` memories,
  `rtr_` retrospectives, `att_` attachments, `img_` images agents read,
  `dlv_` messages, `usr_` people, `rnr_` runners. Paths take a project's or workspace's slug or id,
  and a session's id (`project#number` is the CLI's; it looks the number up
  in `GET /sessions?project=…`).
- **Times** are milliseconds since the Unix epoch.
- **Absent fields**: optional fields are left out rather than sent as
  `null`, and lists that are empty may be left out too.
- **Paging**: lists that page return a `next_before` to pass as `?before=`
  for the next page, and take `?limit=`.

## Errors

A request that fails answers with an HTTP status and a body:

```json
{"code": "not_found", "message": "Session not found"}
```

`code` is for programs, `message` for people. `field` names the request
field a `validation` error is about, and `retry_after` the seconds to wait
after `rate_limited`.

| Status | `code` |
|---|---|
| 400 | `bad_request`, `validation` |
| 401 | `unauthorized` (no token, or one that no longer works), `invalid_credentials` |
| 403 | `forbidden` (not allowed: the role or the token's limits), `access_denied`, `two_factor_setup_required` (the instance requires two-factor sign-in and the account has none yet), `verification_required` (a sensitive action that the token must confirm first: `fctl account verify`) |
| 404 | `not_found`, also for what exists but the caller may not see |
| 409 | `conflict` (a message to a failed session, a resume of one that runs…), `setup_required` |
| 429 | `rate_limited` |
| 501, 503 | `not_configured` (the instance lacks what it needs, such as a GitHub App), `unavailable` (GitHub, a runner) |
| 500 | `internal` |

## Sessions

### Start one

`POST /api/v1/projects/{project}/sessions`, for people who start sessions in
the project and tokens that run sessions there:

```sh
curl -sS -H "Authorization: Bearer $FCTL_TOKEN" -H "Content-Type: application/json" \
  -d '{"prompt": "Fix the flaky login test and open a pull request", "github_access": "push"}' \
  "$FCTL_SERVER/api/v1/projects/my-app/sessions"
```

| Field | What it asks for |
|---|---|
| `prompt` | the task (may be empty when `attachments` has files) |
| `title` | default: the task's first line; the agent renames the session once it understands the work |
| `branch`, `base_branch` | default: `fctl/<number>-<title>`, from the project's default branch |
| `model`, `effort` | default: the project's; `opus`, `sonnet`, `haiku`, `fable` or a model's id, and `low`, `medium`, `high`, `xhigh`, `max` |
| `github_access` | `push`, `comment` or `read`: at most what the project and the token allow, which it gets when absent |
| `github_issues` | `true` to reach GitHub's issues whatever the project's tracker (project maintainers) |
| `attachments` | ids of files uploaded to `POST /api/v1/projects/{project}/attachments` |

It answers with the session, `queued`. A session is:

| Field | What it says |
|---|---|
| `id`, `number`, `project_id`, `project_slug`, `workspace` | who it is: `my-app#12` is `project_slug#number` |
| `title`, `description` | what it works on, as its agent or people say |
| `status`, `status_reason`, `resume_at` | `queued`, `provisioning`, `preparing`, `running`, `idle`, `paused`, `stopping`, `stopped` or `failed`, and why |
| `created_by`, `created_at`, `started_at`, `ended_at`, `last_activity_at`, `updated_at` | who and when |
| `branch`, `base_branch` | its branch and where it started from |
| `pull_requests` | every pull request its agent opened or found: `url`, `number`, `repo`, `title`, `state` (`open`, `draft`, `queued`, `merged`, `closed`), `checks` (`passed`, `failed`, `pending`, and `review`: fctl's review of the newest commit, which the counts leave out, with its `state` (`running`, `passed`, `commented`, `superseded`, `stopped`, `failed`), `waiting` for a runner, `findings`, `still_open` from earlier reviews, `session_id`, its summary's `url` and the `project_id` whose review agents reviewed it, where `POST /api/v1/projects/{id}/reviews` reviews it again) |
| `model`, `effort`, `github_access`, `github_issues` | what it runs with |
| `stats` | turns, tool calls, changed files, additions and deletions, tokens, context |
| `role`, `following`, `archived` | the caller's role in it, whether they follow it, and whether it is archived |
| `schedule`, `ticket`, `review` | the schedule, ticket or reviewed pull request it started from |
| `remote_control` | its Remote Control link, for the person it runs for |

### Read them

- `GET /api/v1/sessions`: the sessions the caller sees, newest first,
  without review agents' and archived ones. `?project=`, `?workspace=`,
  `?status=active` (queued to stopping), `?status=ended` (stopped or
  failed) or one status, `?mine=1` (started by the caller), `?reviews=1`,
  `?archived=1`, `?limit=` (50 unless set, at most 200), `?before=` (a
  `created_at`). Answers `{"sessions": […], "next_before": …}`.
- `GET /api/v1/sessions/{id}`: one session.
- `GET /api/v1/search?q=…`: the sessions whose title, description,
  messages or agent's replies have the words, with `?workspace=`,
  `?project=` and `?limit=` (20, at most 50). Each result has the `session`,
  up to three `matches` (`kind`, `seq`, `text`, `ranges`) and a `count`.

### Message, interrupt, stop, resume

| Request | Body | What it does |
|---|---|---|
| `POST /api/v1/sessions/{id}/messages` | `{"text": "…", "client_id": "…", "attachments": [...]}` | reaches the agent at its next step; resumes a stopped session on the subscription it ran on; `409` for a failed one. Answers `{"delivery_id": "dlv_…"}` |
| `POST /api/v1/sessions/{id}/interrupt` | | stops the current turn |
| `POST /api/v1/sessions/{id}/stop` | | ends the session; its sandbox stays for a resume |
| `POST /api/v1/sessions/{id}/resume` | `{"prompt": "…", "attachments": [...]}` | starts a stopped or failed session again, on the caller's subscription; `409` while it runs |
| `PATCH /api/v1/sessions/{id}` | `{"title": "…", "description": "…", "archived": true}` | renames, describes or archives it (`""` clears the description) |

`client_id` is a key of your own: a message sent again with the same one
is delivered once, and answers with the same `delivery_id`, so retry with
it after a timeout. Files for a message go to `POST
/api/v1/sessions/{id}/attachments` first.

### Events

`GET /api/v1/sessions/{id}/events` returns a page of the transcript,
`{"events": […], "has_more": true}`, oldest first:

- `?after=<seq>`: the events after that one (from the start without it);
  page on with the last `seq` while `has_more`.
- `?before=<seq>`: the events before that one, the newest of them.
- `?limit=`: 200 unless set, at most 1,000.

An event is `{"seq": 21, "ts": …, "kind": "…", "data": {…}}`. The kinds are
`user_message`, `claude` (Claude Code's stream-json messages as it wrote
them: `assistant`, `user`, `system`, `result`, `rate_limit_event`),
`status`, `git`, `log`, `notice`, `remote_control` and `interrupt`;
[fctl for agents](agents.md#reading-a-transcript) says what each holds. A
turn ends with a `claude` event whose `data.type` is `result`, whose
`data.result` is the agent's last message.

Images in `claude` events come without their bytes (`omitted_bytes` in
the image's `source` says how many there were). The session keeps a copy
of each image the agent's tools returned, such as Claude Code's `Read` of
a screenshot: a PNG, JPEG, GIF or WebP of up to 5 MB, for 7 days. Its
`source.kept` says which (`id`, `media_type`, `size`, `width`, `height`,
`expires_at`), and `GET /api/v1/sessions/{id}/images/{image}` returns it to
anyone who can see the session until it expires, `404` after.

To wait for the agent's reply to a message, note the newest `seq` before
sending it, then read the events after it, page by page, until one is a
`result`:

```sh
API=$FCTL_SERVER/api/v1
auth=(-H "Authorization: Bearer $FCTL_TOKEN")
last=$(curl -sS "${auth[@]}" "$API/sessions/$id/events?before=999999999&limit=1" | jq '.events[-1].seq // 0')
curl -sS "${auth[@]}" -H "Content-Type: application/json" \
  -d '{"text": "Also add a regression test", "client_id": "nightly-41-tests"}' "$API/sessions/$id/messages"
reply=
while [ -z "$reply" ]; do
  page=$(curl -sS "${auth[@]}" "$API/sessions/$id/events?after=$last&limit=1000")
  reply=$(jq -r '[.events[] | select(.kind == "claude" and .data.type == "result")] | last | .data.result // empty' <<<"$page")
  last=$(jq --argjson last "$last" '[.events[].seq] | max // $last' <<<"$page")
  [ -n "$reply" ] || [ "$(jq .has_more <<<"$page")" = true ] || sleep 20
done
echo "$reply"
```

Check `GET /api/v1/sessions/{id}` too while you wait: a `failed` session
takes no more turns, and a `paused` one waits for its person's usage limit
to reset (`resume_at`).

### Follow it live

`GET /api/v1/sessions/{id}/ws` opens a WebSocket (`?after=<seq>` after a
reconnect) with the same `Authorization` header. Frames are JSON with a
`type`:

- From the server: `snapshot` first (the session, the caller's `role`, the
  latest `events`, `has_more_before`, the response in progress in `live`,
  who is watching, whether the runner is connected), then `events` (new
  stored events), `stream` (Claude's streamed deltas, not stored),
  `session` (the summary changed), `presence`, `runner_connected`,
  `result` (the outcome of a client frame, by its `client_id`), `error`
  and `pong`.
- From the client: `send` (`client_id`, `text`, `attachments`),
  `interrupt` (`client_id`), `ping` (the apps send one every 25 seconds; a
  viewer that stops pinging is closed after two minutes), `typing`, and
  `visibility` (`{"type": "visibility", "visible": false}`: a viewer counts
  as watching, so its person gets no notifications about the session, until
  it says otherwise).

The socket stays open when the session stops: close it yourself. The server
closes it with 4403 once the caller may no longer see the session.

### Share and follow

- `GET /api/v1/sessions/{id}/shares`; `POST` (`{"email": "…", "role":
  "viewer" | "editor" | "owner"}`, someone in its workspace, by the address
  they joined it as) and `DELETE /api/v1/sessions/{id}/shares/{user}` for
  the session's owners. Only the person who started it, or a workspace
  admin, shares ownership.
- `PUT` and `DELETE /api/v1/sessions/{id}/follow`: the caller's
  notifications about it.
- `GET /api/v1/sessions/{id}/retrospectives`: what its agent recorded.

## Schedules

| Request | What it does |
|---|---|
| `GET /api/v1/projects/{project}/schedules` | `{"schedules": […]}`: each with `id`, `name`, `prompt`, `cron` or `at` (or neither: runs when asked), `timezone`, `model`, `effort`, `base_branch`, `github_access`, `enabled`, `state` (`active`, `paused`, `done`), `paused_reason`, `problem`, `next_run_at`, `last_run`, `owner` |
| `POST /api/v1/projects/{project}/schedules` | `{"name", "prompt", "cron" or "at", "timezone", "model", "effort", "base_branch", "github_access", "enabled": false}`: set one up, on the caller's subscription (people who start sessions in the project; tokens with `full` access) |
| `GET`, `PATCH`, `DELETE /api/v1/schedules/{id}` | read, change (absent fields stay; `""` for the project's defaults; `"enabled": false` pauses it; `"run_as_me": true` takes it over), delete |
| `GET /api/v1/schedules/{id}/runs` | `{"runs": […], "next_before": …}`, newest first |
| `POST /api/v1/schedules/{id}/runs` | `{"context": "…"}` (at most 20,000 characters): run it now, on the caller's subscription |

A run is `{"id": "srn_…", "outcome": "started" | "skipped" | "failed",
"reason", "trigger": "time" | "person" | "token", "by", "session": {"id",
"number", "title"}, "status"}`. Runs that a token asks for give the agent
the context after the task, marked as information from outside fctl, not
instructions: send what another tool knows there, never in a task.
People and tools run one schedule at most 20 times an hour, and a paused
schedule runs not even then. [Automation](automation.md#start-agents-from-other-tools)
shows a tool's request.

## Projects

| Request | What it does |
|---|---|
| `GET /api/v1/projects` | the caller's projects (`?workspace=`) |
| `GET /api/v1/projects/{project}` | one, with its primary `repo`, `default_branch`, `settings`, session counts and the caller's `role` |
| `POST /api/v1/projects` | `{"workspace", "name", "slug", "description", "repo": "owner/name"` or `"create_repo": {"name", "private", "owner"}`, `"settings"}`: workspace admins |
| `PATCH /api/v1/projects/{project}` | `{"name", "description", "repo", "default_branch", "settings", "archived"}`: maintainers |
| `DELETE /api/v1/projects/{project}` | with its sessions, once none of them is active (`409` until then): owners. It can't be undone |
| `GET`, `POST /api/v1/projects/{project}/repos`, `DELETE …/repos/{owner}/{name}` | the repositories its agents use: `{"repo": "owner/name" or "owner/*", "access": "read" | "write"}` |
| `GET`, `POST /api/v1/projects/{project}/members`, `PATCH`, `DELETE …/members/{user}` | its members: `{"email", "role": "owner" | "maintainer" | "editor" | "viewer"}` (maintainers) |
| `GET /api/v1/projects/{project}/secrets`, `PUT`, `DELETE …/secrets/{name}` | secrets as environment variables of its sandboxes: `PUT` takes `{"value": "…"}`; the list has names and who set them, never values (maintainers) |
| `GET /api/v1/projects/{project}/tickets` | its ticket tracker as it works now, and the sessions its tickets started |

`default_branch` is where sessions start unless they ask for another
branch (`base_branch`), so the repository must have it, unless it has no
commits yet. On an instance with a GitHub App, the control plane asks
GitHub when a `PATCH` changes it or `repo`, and when a `POST` that links a
repository names one, and refuses a branch the repository lacks with a
`validation` error on `default_branch`. A new repository (`create_repo`)
has only the branch GitHub makes, which becomes the default, so a `POST`
with both is refused. A session whose base branch the repository lacks
fails, and its `status_reason` says so.

Without a `slug`, a new project's comes from its `name`, with a random
ending of six hex digits (`my-app-829d34`) when another project has that
one or it is reserved, by fctl or by an instance admin
(`/api/v1/admin/reserved-slugs`). A `slug` that is taken or reserved is
refused (`400`, `field: "slug"`). Workspaces' slugs work the same way.

`settings` in a `PATCH` replaces all of the project's settings, so send
them all: read the project, change what you need, and send its `settings`
back, as `fctl projects set` does (`fctl projects set -p my-app
--instructions "Write the tests first." --model opus --effort high` is
the same as this):

```sh
API=$FCTL_SERVER/api/v1
auth=(-H "Authorization: Bearer $FCTL_TOKEN" -H "Content-Type: application/json")
settings=$(curl -sS "${auth[@]}" "$API/projects/my-app" |
  jq '.settings | .instructions = "Write the tests first." | .model = "opus" | .effort = "high"')
curl -sS "${auth[@]}" -X PATCH -d "{\"settings\": $settings}" "$API/projects/my-app"
```

The settings: `setup_script`, `instructions` (what agents are told after
fctl's own), `model`, `effort`, `image` or `image_build`, `cpus`,
`memory_mb`, `package_caches`, `env` (not secret), `idle_timeout_minutes`,
`auto_pull_request` (agents open a pull request when they finish),
`pull_request_descriptions` (`brief`, `balanced`, `full`),
`pull_request_instructions` (what agents are told about handling pull
requests and issues: `null` for fctl's defaults, `""` for nothing),
`github_access`, `follow_ups` (`comments`, `checks`, `conflicts`,
`closed`), `tickets`, `remote_control`, `claude_ai_connectors`,
`credit_people`, `credit_claude`, `publish_screenshots`,
`screenshot_days`, `project_memory` and `retrospectives`
([`ProjectSettings`](../crates/fctl-proto/src/session.rs) describes each).

### Memory, retrospectives and reviews

| Request | What it does |
|---|---|
| `GET /api/v1/projects/{project}/memories` | `{"memories": […], "current": n}`: current notes first, newest first, then stale ones; `?session=` for one session's |
| `POST /api/v1/projects/{project}/memories` | `{"text": "…", "replaces": "mem_…", "also_replaces": [...]}`: a note for later agents (editors; tokens that run sessions) |
| `PATCH`, `DELETE /api/v1/memories/{id}` | `{"text", "stale": true, "reason", "duplicate_of"}`: change, mark stale or current again, delete (editors) |
| `GET /api/v1/projects/{project}/retrospectives` | newest first, `?outcome=completed` / `partial` / `blocked`, `?limit=`, `?before=`; the first page counts `outcomes` |
| `GET`, `PATCH /api/v1/projects/{project}/review-agents` | its review agents: `{"enabled", "run_as_me", "model", "effort", "instructions", "bots", "skipped_repos", "description_summary"}` (owners turn them on, on their own subscription); `repos` are the repositories they review, `skipped_repos` those they don't (`["acme/shared-lib"]`, `[]` for none); `description_summary`: their agents sum each pull request up at the end of its description before the review (off by default) |
| `GET /api/v1/projects/{project}/reviews` | its reviews, newest first: `state` (`running`, `passed`, `commented`, `superseded`, `stopped`, `failed`), the pull request, `findings`, `review_url`, `opened_by` (`id`, `number`, `title`): the project's session whose agent opened the pull request, if one did, and `pull_request_summary` (`risk`: `low`, `medium`, `high` or `critical`, `risk_reason`, `overview`, `highlight`): what the agent put in the pull request's description, where it did; `?session=` for one session's review |
| `POST /api/v1/projects/{project}/reviews` | `{"pull_request": "12"}` (or `owner/name#12`, or its URL): review its head now |

## Everything else

The rest of the API is what the web app and the CLI use for people,
workspaces, runners, Claude connections, notifications and administration:
`/api/v1/workspaces/…`, `/api/v1/runners/…`, `/api/v1/me/…`,
`/api/v1/admin/…`, `/api/v1/auth/…`. Access tokens with limits reach none
of it. The routes are listed in
[`crates/fctl-control/src/lib.rs`](../crates/fctl-control/src/lib.rs), and
[`crates/fctl-client/src/api.rs`](../crates/fctl-client/src/api.rs), the
CLI's client, calls each with its types.
