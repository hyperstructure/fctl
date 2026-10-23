# The CLI

`fctl` does everything the web app does, for people and for scripts and
agents. Every command takes `--json` for machine-readable output and
`--server <url>` (or `FCTL_SERVER`) to pick an instance; `fctl <command> --help`
has the details.

## Signing in

```sh
fctl login app.fctl.sh       # opens the browser to approve this device
fctl login --no-browser      # prints the link and code instead (SSH, servers)
fctl whoami
fctl logout
```

You approve the device in the browser, signed in there, so two-factor
sign-in happens in the browser too. `fctl login` opens only a link on the
server it signs in to: a server that sends you to approve the sign-in on
another site could be relaying that site's code to get a token there, so
it stops and says which site to sign in to instead.

For scripts, CI and bots, create an access token, limited to what it is
for, and pass it as `FCTL_TOKEN`:

```sh
fctl token "PR reviews" --access run -p my-app --github comment --days 90
FCTL_TOKEN=fctl_pat_… fctl run -p my-app --github comment -d "Review pull request #12"
```

### Access tokens

`fctl token` prints a new token once. Without `--access`, `-p`, `-w` and
`--github` it does everything you can, as you do when signed in; with any,
it is limited, and reaches projects and sessions only: never your account,
your workspaces' people and settings, your Claude connections or your other
tokens.

```sh
fctl token "Dashboard" --access read -w acme   # reads every project of acme, those made later too
fctl token "Nightly" --access run -p my-app    # starts and steers sessions in my-app
fctl token "Admin script" --access full -p my-app   # everything you can do in my-app
fctl token "PR reviews" --access run -p my-app --github comment --days 90
fctl tokens                                    # your tokens and what each does, and where you are signed in
fctl tokens revoke "PR reviews"                # by name or id (tok_…), at once
```

`--access` says what it does: `read` reads projects, sessions, transcripts,
memory and retrospectives and changes nothing; `run` also starts sessions,
messages and interrupts their agents, stops and resumes the sessions it
started, and keeps projects' memory; `full` does all you can in its
projects. `-p` and `-w` (repeat them) keep it to some projects, or to every
project of some workspaces. `--github` is the most the agents of its
sessions may do on GitHub (below), for a token that runs sessions (it makes
one, without `--access`): sessions it starts get that unless they ask for
less, and it can't message or resume a session whose agent started able to
do more. A token that does everything can't take it, since it could change
the settings that steer its projects' agents. Sessions it starts don't notify you; follow the ones you want.
`fctl whoami` with a token says what it does. [Automation](automation.md)
runs review agents from GitHub Actions with one.

Configuration lives in `~/.config/fctl/` (`config.toml`, and
`credentials.toml` with mode 0600). `FCTL_CONFIG_DIR` moves it.

## Sessions

Sessions are named by their id (`ses_…`), their URL, or `project#number`.

```sh
fctl run -p my-app "Add a dark mode toggle"   # start one and follow it
fctl run -p my-app -d -f task.md              # start from a file, don't follow
echo "Fix the flaky test" | fctl run -p my-app
fctl run --model opus --effort high "…"       # override the project's agent settings
fctl run --github comment "Review pull request #12"   # its agent reviews and comments, and pushes nothing
fctl run --github read "Plan the move to Postgres"    # its agent only reads on GitHub
fctl run --branch fix/login --base develop "…"
fctl run -p my-app -a screenshot.png "Make the header match this"   # attach files (repeat -a)

fctl sessions                    # recent sessions
fctl sessions --active -p my-app
fctl show my-app#12              # status, branch, pull requests and their checks, model and effort, usage
fctl edit my-app#12 --title "Fix the flaky login test"
fctl edit my-app#12 --description "Retries the redirect and tests it."   # "" clears it
fctl logs my-app#12              # the transcript
fctl logs -f my-app#12           # …and keep following

fctl attach my-app#12            # stream it and type messages (Ctrl-C detaches)
fctl attach --read-only my-app#12
fctl send my-app#12 "Use the existing Button component"   # a stopped session resumes for it
fctl send my-app#12 -a ci.log "This is what CI printed"
fctl interrupt my-app#12         # stop the current turn
fctl stop my-app#12              # end the session (its sandbox is kept for resuming)
fctl resume my-app#12 "Now add tests"   # also a failed one, on your Claude subscription
fctl follow my-app#12            # get its notifications (see Notifications)
fctl unfollow my-app#12
fctl open my-app#12              # in the browser
fctl open my-app#12 --claude     # in the Claude app (Remote Control)
```

`fctl run` uses the project from `fctl projects use` when `-p` is missing.

A session's agent pushes, comments or only reads on GitHub (`--github`):
one that pushes commits and pushes its branch, as agents always did; one
that comments reviews, comments and labels, with a token that can't push;
one that reads writes nothing. It gets as much as its project's settings
and the token that starts it allow unless it asks for less, and `fctl
show` says what it got. GitHub lets only tokens that can push resolve
review threads, so an agent that comments resolves them through fctl, in
its sandbox:

```sh
fctl resolve PRRT_kwDOABCD1234         # in a session's sandbox: resolve a review thread
fctl resolve --unresolve PRRT_kwDOABCD1234
```

`fctl attach` also says when the session's runner goes away or can't reach
the agent in its sandbox, and when it is back.

A session's title starts as the first line of its task. Agents rename it to
what they work on, and say what the session is about, with the same
command: in a sandbox, `fctl edit --title … --description …` without a
session edits the one the sandbox runs, through fctl-agent and without a
sign-in. The sandbox's `~/AGENTS.md` asks them to do it once they
understand the task, and again when the work turns to something else.

`--attach` (`-a`) sends files with the task or message (`run`, `send`,
`resume`): up to 10, of at most 20 MB each. The agent finds them in
`~/attachments/` in its sandbox, and sees images that Claude accepts as
they are (PNG, JPEG, GIF, WebP up to 2000 px a side) in the message itself.

### Screenshots in pull requests

GitHub takes uploads only from people, so in a sandbox `gh pr edit
--attach` fails with "unsupported authentication type": sessions have the
GitHub App's token. In projects that publish screenshots, agents publish
them on fctl instead and put the Markdown it prints in the pull request:

```sh
fctl publish before.png after.png     # in a sandbox: a link and Markdown for each
fctl publish --json after.png         # the links, sizes, expiry and Markdown
fctl unpublish https://app.fctl.sh/api/v1/published/…/after.png   # delete one sooner
```

Anyone who has a link can open it, until it expires: after 30 days, or
what the project chooses between 24 hours and 60 days. A link holds 256
random bits. Publishing is off until the instance's admins allow it (`fctl
admin set --publish-screenshots on`) and a project's maintainers turn it
on (`fctl projects set --publish-screenshots on`); turning either off
takes the screenshots offline. It takes PNG, JPEG, GIF and WebP images of
at most 5 MB, up to 100 per session. The transcript shows each one, and
people who manage the session can delete it there or with `fctl
unpublish`.

## Workspaces

Projects, the runners that run their sessions and the GitHub accounts they
work in belong to workspaces. `fctl workspaces` (or `fctl workspace`) lists
yours and the invitations waiting for you; the commands below take the
workspace with `-w`, else the one from `fctl workspaces use`, else your only
one.

```sh
fctl workspaces                            # yours, and the invitations waiting for you
fctl workspaces create "Side project" --color pink
fctl workspaces create Acme --as ada.lovelace@acme.example   # join it as another of your addresses
fctl workspaces show -w side-project
fctl workspaces use side-project           # the default for the commands below
fctl workspaces order side-project acme    # yours in this order, in lists and the app's sidebar
fctl workspaces join side-project          # an invitation (or decline it)
fctl workspaces leave side-project         # your sessions there stop
fctl workspaces claude crd_01… -w acme     # your sessions there run on this Claude connection
fctl workspaces claude ada@acme.example -w acme   # …named by its account or label
fctl workspaces claude on-runners -w acme  # …on the token you keep on your runners
fctl workspaces claude default -w acme     # …or on your default one
fctl workspaces own-runners on -w acme     # your sessions there run only on runners you registered
fctl workspaces own-runners off -w acme    # …or on any of its runners for everyone's sessions too

fctl workspaces members                    # its people, and the invitations nobody answered
fctl workspaces invite grace@example.com --admin
fctl workspaces uninvite grace@example.com
fctl workspaces role grace@example.com member
fctl workspaces remove grace@example.com   # their sessions there stop
fctl workspaces transfer grace@example.com # the primary admin hands the role to an admin

fctl workspaces set --name "Side project" --avatar logo.png   # or --avatar none
fctl workspaces set --base-role editor     # every member starts sessions in every project
fctl workspaces set --require-two-factor on   # closed to members without a second factor
fctl workspaces set --skip-invites on      # a workspace for one: its setup stops asking for people

fctl workspaces runners                    # the runners its sessions start on, with their ids
fctl workspaces add-runner studio-mac      # one of yours runs its sessions too (its name or id)
fctl workspaces add-runner                 # on a runner: this machine
fctl workspaces add-runner studio-mac --personal   # only your sessions there (a member's always)
fctl workspaces remove-runner studio-mac   # the ones running there stop and resume elsewhere

fctl workspaces github                     # the GitHub accounts it works in
fctl workspaces connect                    # opens a GitHub sign-in; you choose what it works in on its GitHub tab
fctl workspaces connect acme               # the same, for acme only
fctl workspaces connect acme --repo web --repo design   # only these repositories of it, without the choice
fctl workspaces disconnect acme

fctl workspaces audit                      # what happened in it (admins)
fctl workspaces delete side-project        # with its projects and sessions; asks you to type its name
```

Admins invite people by an address they sign in with, or one the allowlist
admits (they join when they sign up); any other address needs an instance
invitation first (`fctl admin invite`).
Everyone in a workspace works in its projects as their project role or its
base role says; its admins own every project. One person can be in at most
five workspaces. While the instance requires two-factor sign-in (`fctl admin
set --require-two-factor on`), every workspace does, and `fctl workspaces
set --require-two-factor off` fails.

A runner its admins add runs everyone's sessions there; one a member adds,
and one an admin adds with `--personal`, runs only its owner's. With
`own-runners on`, your sessions in the workspace run only on runners you
registered, not on its other people's, which see a session's code, secrets
and Claude token; turning it on stops the ones running elsewhere, and they
resume on yours.

## Projects

```sh
fctl projects list                     # all your workspaces' projects
fctl projects list -w side-project
fctl projects show my-app
fctl projects create "My app" --repo my-org/my-app -w acme   # a workspace's admins create its projects
fctl projects create "New thing" --new-repo new-thing --owner my-org --public
fctl projects create "Site" --repo my-org/site --connectors   # claude.ai connectors in its sessions (off by default)
fctl projects set -p my-app --credit-claude off   # no Claude trailer in its commits
fctl projects set -p my-app --credit-people off   # no trailers for the people who start sessions
fctl projects set -p my-app --idle-timeout 30     # idle sessions keep their sandbox 30 minutes (default 10)
fctl projects set -p my-app --publish-screenshots on --screenshot-days 7   # links work for a week
fctl projects set -p my-app --image ghcr.io/acme/agent:1   # sessions start from this image ("" for the runner's)
fctl projects set -p my-app --build .devcontainer/devcontainer.json   # or one built from the repository
fctl projects set -p my-app --build Dockerfile --build-context app    # a Dockerfile, built in app/
fctl projects set -p my-app --setup-script 'npm ci'   # or @setup.sh, or - to read it from stdin
fctl projects set -p my-app --package-caches on    # sessions on a runner share their downloads
fctl projects set -p my-app --env NODE_ENV=test --unset-env DEBUG
fctl projects set -p my-app --cpus 8 --memory 16g     # "default" for the runner's
fctl projects set -p my-app --project-memory off --retrospectives off   # agents keep neither
fctl projects set -p my-app --github comment      # its agents review and comment at most, and push nothing
fctl projects set -p my-app --pr-descriptions brief   # pull requests described in a few sentences (balanced by default, or full)
fctl projects set -p my-app --follow-ups comments,checks,conflicts   # agents come back for @fctl comments, failed checks and conflicts
fctl projects use my-app
fctl projects order my-app site        # these first in their workspace, the others after them
fctl projects delete my-app            # with its sessions; asks you to type the slug
```

Agents start in the project's repository and can use more of the same GitHub
account. They clone them when they need them, into `~/repos/<owner>/<name>`;
in each one they change, they work on the session's branch, which fctl
pushes after every turn until its pull request is merged, and open a pull
request.

```sh
fctl projects repos list -p my-app
fctl projects repos add my-org/my-lib --write -p my-app   # agents may push and open pull requests
fctl projects repos add my-org/design -p my-app           # read only, like a private dependency
fctl projects repos add 'my-org/*' -p my-app              # read every repository the GitHub App reaches
fctl projects repos remove my-org/design -p my-app
```

Changes apply to sessions that start afterwards.

Follow-ups bring agents back to the pull requests they opened, in a
message from fctl that resumes the session on the Claude subscription it
ran on (`--follow-ups`, a list of `comments`, `checks`, `conflicts` and
`closed`, or `all` or `none`): a comment that mentions `@fctl` or
`@agent`, or starts with `fctl:` or `agent:`, from someone who can write
to the repository; a failed check of the newest commit, at most three
times in a row while nobody messages the agent; conflicts with the base
branch, which the agent resolves by merging the base branch in, as often;
a merge or a close. New projects start with all four on; `fctl projects
show` says which ([Architecture](architecture.md#follow-ups)).

A project's image and its build run only on runners whose owners allow
them ([Sandbox images](sandbox-images.md#which-images-a-host-runs)); a
session that no runner allows stays queued and says why. `fctl projects
show` says where the project's sessions start from, their size, setup
script and variables.

With `--package-caches on`, the project's sessions on each runner share
what npm, pnpm, Yarn, Bun, Cargo, uv, pip, Go and Playwright download, so
a setup script like `npm ci` or `cargo fetch` gets only what is new. The
runner keeps them for the project up to its owner's limit, 10 GiB unless
`host.toml` says otherwise, empties them past it, and removes them two
weeks after a session last used them ([Hosts](#hosts)). Sessions get them
on runners with the Docker or Apple container driver (and the process
driver), not Lima, and review agents never do. A session in a sandbox
made before the project turned them on gets them in its next sandbox.

### Memory and retrospectives

A project keeps a memory: notes its agents, and its people, write for the
agents of its later sessions, about what the repository doesn't say. Each
session's agent starts with the current notes, adds what a future agent
should know and marks the notes that turned out wrong stale; everyone in
the project reads them, and editors change and delete them:

```sh
fctl memory -p my-app                  # the current notes, newest first, with where each came from
fctl memory -p my-app --stale          # the stale ones too
fctl memory show mem_01JC3V…
fctl memory add -p my-app "Deploys happen on Fridays: merge migrations by Wednesday."
fctl memory add -p my-app "Deploys happen on Thursdays." --replaces mem_01JC3V…   # the old one goes stale
fctl memory add -p my-app "…what both say…" --replaces mem_01JC3V…,mem_01JC2Q…     # in place of both
fctl memory edit mem_01JC3V… "Deploys happen on Thursdays."
fctl memory stale mem_01JC3V… --reason "We deploy every day now"   # agents no longer start with it
fctl memory duplicate mem_01JC3V… --of mem_01JC2Q…   # stale, linked to the note it repeats
fctl memory restore mem_01JC3V…        # current again
fctl memory rm mem_01JC3V…             # for good
```

As agents finish a piece of work, they record a retrospective of it: how
it ended, what they worked on, the approach they took, what got in the way
and what a future agent should know. One for each piece of work, not for
each turn: when a later turn carries on with it, the agent updates the one
it recorded. Session pages show them, and the project's Retrospectives tab
lists them all:

```sh
fctl retro list -p my-app              # newest first, with how many ended each way
fctl retro list -p my-app --outcome blocked
fctl retro list my-app#12              # one session's
fctl retro rm rtr_01JC3W…              # people who manage the session
```

In a session's sandbox, the same commands without `-p` are the agent's,
through fctl-agent and without a sign-in, as for `fctl edit`. `~/AGENTS.md`
lists the project's notes and says how to keep them, and when to record a
retrospective and when to update one:

```sh
fctl memory                            # the notes as they are now, other sessions' new ones too
fctl memory add "The e2e tests need Chromium: run npx playwright install chromium first."
fctl memory add "Run cargo xtask check before pushing." --replaces mem_01JC3V…   # the old one goes stale
fctl memory stale mem_01JC3V… --reason "The script moved to xtask"
fctl memory duplicate mem_01JC3V… --of mem_01JC2Q…   # it repeats a current note
fctl retro --outcome completed --worked-on "Retried failed uploads (#41)." \
  --approach "Used the existing backoff helper." --challenges "The tests need MinIO." \
  --insights "Start MinIO with docker compose up minio before the tests."
fctl retro list                        # this session's, with their ids
fctl retro update rtr_01JC3W… --outcome completed \
  --challenges "The tests need MinIO, which CI lacked until #42."   # the parts left out stay
```

Agents can't change or delete notes, only mark them stale with a reason,
or as duplicates of current notes. They update only their own session's
retrospectives, and delete none. A note replaces at most 20 others.
Both are on in new projects and projects from before them; `fctl projects
set --project-memory off` and `--retrospectives off` turn them off, and the
notes and retrospectives stay for people to read. A project keeps at most
500 notes of 2,000 characters, stale ones included, and a session at most
50 retrospectives.

### Review agents

Where a project's review agents are on, each new commit of a pull request
in the repositories its agents may write to gets a review: an agent reads
it, on the Claude subscription of the owner who turned them on, and fctl
posts what it found on the pull request, with a check on the commit.
A pull request from someone who can't write to the repository gets one
only when someone asks for it (`fctl review start`), since what it says
reaches an agent with the owner's Claude token.
When an fctl agent opened the pull request, its session gets the findings
in a message from fctl, decides what to do about them and answers in
their threads. The project's owners turn them on (on their own
subscription), take them over and say how they review; the person they
run for may turn them off:

```sh
fctl review settings -p my-app                 # on or off, on whose subscription, how
fctl review settings -p my-app --on --model opus --effort high
fctl review settings -p my-app --instructions-file review.md   # what they're told besides fctl's own
fctl review settings -p my-app --mine          # run them on your subscription from now on
fctl review settings -p my-app --model default # the project's model again
fctl review settings -p my-app --off           # reviews under way finish
fctl review -p my-app                          # the reviews, newest first, with what came of each
fctl review start -p my-app 12                 # review a pull request's head now (or acme/api#7, or its link)
```

Review sessions take no messages: `fctl show my-app#12` says which pull
request one reviews and what came of it. In a review agent's sandbox,
`fctl review` is the agent's, through fctl-agent:

```sh
fctl review threads                    # fctl's earlier findings on the pull request, with the answers
fctl review submit review.json         # post the review, once: --help shows the file
```

### Schedules

A schedule starts a session for a task on its own: once at a set time,
or again and again on a cron expression in a time zone. Each run is a
session of its own, on the Claude subscription of the person the schedule
runs for, who follows it as they follow the sessions they start. A run is
skipped while the schedule's last session still works on its task, and
while that person's plan is at its limit; one that runs once waits for the
limit to reset instead. People who start sessions in the project set up
schedules, on their own subscription, and run any of them now, on theirs;
a schedule's owner and the project's maintainers and owners change, pause
and delete it, and a change by anyone else runs it on theirs from then on:

```sh
fctl schedules -p my-app                       # its schedules, when each runs next and what its last run did
fctl schedules add "Weekly dependencies" -p my-app --cron "0 9 * * 1" --tz Europe/Stockholm \
  -m "Update the dependencies that have new patch or minor releases, and open a pull request."
fctl schedules add -p my-app --cron "0 8 * * 1-5" --task-file triage.md --github comment
fctl schedules add -p my-app --at 2h -m "Check whether the deploy went out, and say so."
fctl schedules add -p my-app --at 2026-10-30T16:00+01:00 -m "Draft the release notes."
fctl schedules add "Triage an issue" -p my-app --when-asked -m "Triage the issue that comes with this."
fctl schedules show "Weekly dependencies" -p my-app   # by name or id: its task, settings and latest runs
fctl schedules edit "Weekly dependencies" -p my-app --cron "0 7 * * 1" --effort high
fctl schedules edit sch_01JC… --mine           # run it on your subscription from now on
fctl schedules pause sch_01JC…                 # it starts nothing until it's resumed
fctl schedules resume sch_01JC…
fctl schedules run sch_01JC…                   # its task now, in a session on your subscription
fctl schedules run sch_01JC… --context "Issue #41 was just opened: the login page hangs"
fctl schedules runs sch_01JC…                  # the sessions it started, and why it started none
fctl schedules rm sch_01JC…                    # the sessions it started stay
```

`--cron` takes five fields (the minute, the hour, the day of the month,
the month and the day of the week, like `0 9 * * 1-5`) or `@hourly`,
`@daily`, `@weekly`, `@monthly` and `@yearly`, and runs at most every 15
minutes; `--tz` defaults to this machine's time zone. `--at` takes how far
ahead (`2h`, `30m`, `1d`) or a time with its UTC offset. `--when-asked`
sets up one without a time, which runs only when someone runs it, or
another tool does, each run on the subscription of whoever runs it; a
paused schedule starts nothing, not even then. `--model`,
`--effort`, `--github` and `--base` say what its sessions start with,
`default` for the project's. What `--context` says comes after the task;
from an access token, its agent takes it as information, not instructions
([Automation](automation.md#start-agents-from-other-tools)). A project
keeps at most 50 schedules.

## Notifications

A session you follow notifies you when it waits for your reply, pauses on a
usage limit or fails, in every browser where you turned notifications on
(Settings → Notifications in the web app; on iPhone and iPad, fctl added to
the Home Screen). You follow the sessions you start and resume.

```sh
fctl notifications status                   # what notifies you, and the browsers that get it
fctl notifications set --waiting off        # on or off: --waiting, --paused, --failed
fctl notifications set --follow-started off # follow only the sessions you choose
fctl notifications test                     # a test notification to your browsers
fctl notifications remove psh_01…           # stop notifying a browser
```

Several that arrive close together come as one, and at most one a minute.
People who are looking at a session (in the web app with its tab visible,
or with `fctl attach`) aren't notified about it.

## Claude

```sh
fctl claude connect --login      # full sign-in through a host (Remote Control)
fctl claude connect              # paste a token from `claude setup-token`
fctl claude connect --token sk-ant-oat01-…
fctl claude connect --on-runners # run on the token your runners keep, which you never give to fctl
fctl claude status               # your connections, their ids, how much of each plan is used, and which runners keep your token
```

To keep your token on your runners instead of in fctl, run these on each
runner, then `fctl claude connect --on-runners` anywhere. Your sessions on
that connection run only on your runners that keep the token, in every
workspace, and wait while none of them is online:

```sh
claude setup-token               # makes a token that lasts a year (here, or on any machine with Claude Code)
fctl host claude set-token       # keeps it in ~/.config/fctl/claude-token, mode 0600 (asked for, or read from stdin)
fctl host claude status          # whether this runner keeps one, and whether fctl knows
fctl host claude remove-token    # sessions running here keep it until they stop
```

`claude_token_file` in `host.toml` keeps it somewhere else, such as a file
your secrets manager writes.

## Hosts

```sh
fctl host install                # register this machine and start the service
fctl host install --driver lima --capacity 2 --build-image
fctl host install --cpus 4 --memory 8g --max-memory 12g   # sandbox sizes
fctl host install --allow-image 'ghcr.io/acme/*' --allow-build 'acme/*'   # what projects may run and build
fctl host install --workspace acme   # for a workspace you are in (default: the one you administer, or your only one)
fctl host install --workspace acme --personal   # only your sessions there (a member's always)
fctl host install --token fctl_rt_…   # with a runner token from a workspace's Runners tab
fctl host install --no-service   # write the config only (run it yourself)
fctl host run                    # run in the foreground (what the service runs)
fctl host status                 # with the runner's id and sessions, and whether it drains
fctl host doctor                 # check the driver, image, agent binary, connection
fctl host logs -n 200
fctl host image pull             # pull the sandbox image again now (the host does every 6 hours)
fctl host image build            # build it here instead (with Lima: the base VM)
fctl host image build --source ~/src/sandbox   # from a checkout of hyperstructure/sandbox
fctl host claude set-token       # keep your Claude token here, not in fctl (see Claude above)
fctl host restart                # sessions keep running
fctl host drain                  # take no new sessions; the ones here carry on
fctl host drain --wait           # …and wait until none runs here
fctl host undrain                # take new sessions again
fctl host uninstall              # sandboxes are kept; asks first when sessions run
fctl host uninstall --wait       # drain, and uninstall once none runs
fctl host uninstall --force      # stop the sessions here now, then uninstall
```

Drain a host before maintenance: no new session starts on it, and the
sessions it has carry on there. Those that wait for a message stop once
they have waited a minute, and a message resumes each of them on this host,
where its sandbox has Claude's conversation. So the host is empty whenever
no turn runs, and `--wait` returns then. Admins can drain a runner too,
from the runners list or with `fctl admin drain`; whoever drained it,
either can stop the drain.

`fctl host uninstall` stops the service, so it never leaves sessions to
agents nobody sees: when sessions run on the host, it lists them and asks
before it stops them, or, without a terminal, needs `--wait` or `--force`.
Stopped this way, their agents stop and push their work, and a message
resumes each on another runner, in a new sandbox without its conversation.
The runner keeps draining until `fctl host undrain`.

`fctl host install` checks the driver before it registers anything: if the
driver can't run sessions (Apple's `container` or Lima isn't installed, a
service isn't running), it says what to fix and stops. `--driver auto` (the
default) picks the first ready one of apple, docker and lima, never the
process driver, and the driver it checked goes into the host's settings in
`~/.config/fctl/host.toml`:

```toml
server = "https://app.fctl.sh"
name = "studio-mac"
driver = "apple"         # apple | lima | docker | process | auto (picks at every start)
capacity = 4             # concurrent sessions
# image = "ghcr.io/acme/agent:1"   # unset: the published image of the host's version
cpus = 4                 # per sandbox, when the project sets none
memory_mb = 8192         # per sandbox, when the project sets none
max_cpus = 8             # the most one sandbox may have (unset: all there is)
max_memory_mb = 12288
keep_stopped_hours = 72  # then remove a stopped session's sandbox (0 keeps them)
min_free_disk_gb = 20    # disk to keep free for sandboxes (0 keeps none)
max_cache_gb = 10        # package caches per project that shares them (0 keeps none)
keep_caches_days = 14    # then remove the caches of a project with no session here (0 keeps them)
# allowed_images = ["ghcr.io/acme/*"]   # images projects may choose (unset: any)
# allowed_builds = ["acme/*"]           # repositories it builds images from (unset: none)
build_timeout_minutes = 60
max_build_mb = 20480     # the largest image a build may make
```

Sessions start from their project's image or, without one, the host's:
unless `image` sets another, the published image of the host's own version,
`ghcr.io/hyperstructure/sandbox:<version>`, which the host pulls when it
starts. A project's own image runs only if `allowed_images` allows it, or
is unset, and an image built from its repository only if `allowed_builds`
lists the repository. `--allow-image` and `--allow-build` replace the lists
(`--allow-image '*'` for any image, `--allow-build ""` for no builds), and
`fctl host status` shows them. [Sandbox images](sandbox-images.md) has the
details.

A project's Settings → Sandbox can ask for more CPUs or memory than the
host's default. Sessions only start on a host whose maximum covers that;
when none does, they stay queued and say so ("No runner can give this
session 32 GiB of memory (the most is 15.6 GiB)"). Without `max_cpus` and
`max_memory_mb`, the maximum is what the machine has, or with Docker the
Docker daemon's: on macOS that is the VM of OrbStack, Docker Desktop or
Colima, which is usually smaller than the Mac. A default above the maximum
is lowered to it. `fctl host status` shows the sizes the host gives. The
host measures them when it starts and every five minutes, and the control
plane learns them when the host connects, so run `fctl host restart` after
changing `host.toml` or resizing the Docker VM.

Agents are told their sandbox's size in their instructions and in
`FCTL_SANDBOX_CPUS` and `FCTL_SANDBOX_MEMORY_MB`: with Docker, `nproc` and
`free` show the whole VM rather than the sandbox's share, and with Apple's
`container`, `nproc` counts one CPU more than the sandbox may use.

A stopped session's sandbox takes disk on the host until it has been
stopped for `keep_stopped_hours`, or until the disk runs low. With Apple's
`container`, a sandbox's disk image keeps the space of the files its
session deleted (a build of fctl leaves about 5.5 GB) until the host
stops the sandbox: then the host gives that space back to the Mac with
`container clean`. That needs Apple's `container` 1.4 or later. With an
older one, a stopped sandbox keeps the most its session ever stored, and
`fctl host logs` says so each time a sandbox stops. `df` in the sandbox
shows its 512 GiB disk image rather than what the Mac has free.

The host keeps `min_free_disk_gb` free (20 GiB unless `host.toml` says
otherwise) on the disk where its sandboxes are: Apple's `container` app
root, Lima's `~/.lima`, Docker's root directory or, with Docker on macOS,
the disk inside its VM, and the data directory with the process driver. It
measures every 30 seconds (inside Docker's VM every five minutes, since
that starts a container). With less free than that:

- It removes stopped sandboxes before `keep_stopped_hours`, those stopped
  longest first, one at a time, until there is enough free again. A
  message to one of those sessions resumes it in a new sandbox on its
  branch, with what the agent pushed, and Claude starts a new conversation.
- It takes no new sessions meanwhile, only resumes of the sessions whose
  sandboxes it still has. A session that has nowhere else to go stays
  queued and says "Waiting for a runner: the ones online are low on disk".
- `fctl host status`, `fctl host doctor`, `fctl admin runners` and the
  runners list say how much is free.

Sessions that run carry on, and nothing stops them for disk. When no
stopped sandbox is left to remove, the host takes sessions again once
running ones stop and the host removes their sandboxes, or once something
else frees space. Set `min_free_disk_gb = 0` to remove sandboxes by age
only, or lower `keep_stopped_hours` to remove them sooner whatever the disk
has, and run `fctl host restart` after changing either.

For projects that share package caches (`fctl projects set
--package-caches on`), the host keeps what their sessions' package
managers download: with Docker in a volume per project,
`fctl-cache-<project id>`, and with Apple's `container` in a directory of
its data directory, `caches/<project id>`, that its sandboxes mount at
`/var/cache/fctl`. The sweep that removes stopped sandboxes, when the host
starts and every hour, looks after them too:

- It measures the caches that a session used since they were last
  measured, and empties those that take more than `max_cache_gb` (10 GiB
  unless `host.toml` says otherwise). With Docker, measuring and emptying
  a volume run in a short-lived container of the host's image.
- It removes the caches of a project that no session has started with for
  `keep_caches_days` (14 unless `host.toml` says otherwise). Those that a
  stopped sandbox here mounts stay, emptied, until the sandbox goes.
- Below `min_free_disk_gb`, it empties caches, those used least recently
  first, before it removes stopped sandboxes: an empty cache costs the
  next session downloads, a removed sandbox Claude's conversation.

It leaves alone the caches of a project with a session running here, and
a session that starts while the host measures or empties its project's
caches waits for that. `max_cache_gb = 0` keeps no caches: the next sweep
removes them, and sessions download what they need. `fctl host status`
shows what the caches took when last measured.

## Admin

Instance admins manage people and invitations, the GitHub App's
installations, every runner and the instance's settings, and see what each
workspace is and who runs it, but not inside the ones they aren't members
of.

```sh
fctl admin users                           # with their other addresses and how many workspaces
fctl admin invite ada@private.example      # an invitation link (and an email, when set up)
fctl admin invite grace@example.com -w side-project -w acme:admin   # and to your workspaces
fctl admin role grace@example.com admin    # an instance admin, or member again (the primary instance owner)
fctl admin transfer grace@example.com      # the primary instance owner hands the role to an admin
fctl admin disable grace@example.com       # can't sign in; their tokens stop working (or enable)
fctl admin delete-user grace@example.com   # asks you to type their address
fctl admin workspaces                      # and which need a primary admin
fctl admin set-primary side-project grace@example.com   # when its primary admin's account is disabled
fctl admin delete-workspace side-project   # asks you to type its name
fctl admin github                          # the app's installations, and the workspaces that use each
fctl admin github remove acme side-project               # take acme from the workspace (its admins reconnect it)
fctl admin early-access                    # who asked for early access, where the instance takes requests
fctl admin early-access remove spam@example.com   # asking again adds them back
fctl admin allowlist
fctl admin allow ada@example.com           # also creates an invite link
fctl admin allow '*@example.com'           # a whole domain
fctl admin allow grace@example.com --admin
fctl admin disallow '*@example.com'        # its addresses lose access (and accounts with no other)
fctl admin runners                         # with their ids and images; draining ones say who drained them
fctl admin drain studio-mac                # no new sessions start on it (name or id)
fctl admin undrain studio-mac
fctl admin audit                           # what happened on the instance, newest first
fctl admin settings                        # what the instance allows and requires
fctl admin set --publish-screenshots on    # projects may publish screenshots
fctl admin set --require-two-factor on     # everyone who signs in with a password needs it
fctl admin set --two-factor-google on      # Google sign-ins ask for codes too
fctl admin set --verify-sensitive-actions on   # sensitive actions take a passkey or a code (the primary instance owner)
fctl admin reset-two-factor grace@example.com   # they lost their phone and recovery codes
```

`fctl admin users` says who has two-factor sign-in. With it required, the
command that requires it lists who hasn't set it up yet: until they do,
they can only set it up, in the browser or with `fctl account two-factor
enable`, and their other requests fail.

Where the instance takes requests for early access (as app.fctl.sh does),
`fctl admin early-access` lists the people who asked on the sign-in page
and have no account yet, those waiting longest first, then those invited
and whether their link still works. `fctl admin invite` with their address
answers one: it prints their link, and the request shows as invited.

`fctl admin audit` and `fctl workspaces audit` show 100 entries at a time;
`--before` with the time after "Older:" (`next_before` in `--json`) shows
the ones before them.

## Your account

```sh
fctl account emails                        # your addresses, their commit emails and workspaces
fctl account set --commit-email 1234+ada@users.noreply.github.com      # your primary address's
fctl account set --for ada.lovelace@acme.example --commit-email ada.lovelace@acme.example
fctl account set --commit-email ""         # commits stop crediting you
fctl account primary ada.lovelace@acme.example   # where fctl writes to you
fctl account disconnect ada@private.example      # it stops signing in; you leave its workspaces
fctl account export                        # your data as JSON: fctl-export-<date>.json
fctl account export -o - | jq .sessions    # or print it
fctl account delete                        # shows what goes, asks for your email address
```

Two-factor sign-in takes a code from an authenticator app after your
password, and after Google if you choose:

```sh
fctl account two-factor                    # what you have, and what the instance requires
fctl account two-factor enable             # prints a QR code, asks for the app's code, shows recovery codes
fctl account two-factor enable --code 123456   # replace your app: a code from it, or a recovery code
fctl account two-factor recovery-codes     # ten new ones; the old ones stop working
fctl account two-factor google on          # Google sign-ins ask you for a code too
fctl account two-factor disable            # asks for a code (not while the instance requires it)
```

`2fa` is short for `two-factor`. Without a terminal, `enable --json` prints
the setup (the key, the `otpauth://` link and the QR code's rows), and
`fctl account two-factor confirm --setup … --code …` finishes it.

Where the primary instance owner turns it on, sensitive actions (inviting
and promoting admins, turning two-factor sign-in off, adding or
registering a runner for a workspace (`fctl host install` too), new access
tokens, exporting your data, and deleting accounts,
projects and workspaces) ask you to confirm it's you first. In a terminal
fctl asks by itself, and then does what you asked: for a code from your
authenticator app, or, with a passkey, to approve a code in the browser,
as `fctl login` does. The CLI is then confirmed for 15 minutes. Without a
terminal, confirm first:

```sh
fctl account verify                        # asks for a code, or opens the browser for your passkey
fctl account verify --code 123456          # a code from your authenticator app, or a recovery code
```

Passkeys sign you in with your fingerprint, face or screen lock. Your
browser makes them, in Settings → Passkeys; the CLI lists and manages them:

```sh
fctl account passkeys list
fctl account passkeys rename "Chrome on macOS" "Studio laptop"   # by name or id
fctl account passkeys remove pky_01…
```

Each address is invited to the instance on its own; open an invitation
while signed in to connect its address to your account (the web app asks).
Every address signs in, and each has its own commit email: the commits in a
workspace's sessions credit the one of the address you joined it as.

With a commit email, the commits in the sessions you start end with a
`Co-authored-by` trailer that has your name and that address, for everyone
who can read the repository. Use an address your GitHub account has, such
as its noreply address (GitHub → Settings → Emails), so GitHub links the
commits to you. A project's maintainers can turn this off for its sessions
(`fctl projects set --credit-people off`). Sessions take a change when they
next start or resume; `fctl whoami` shows the address.

The export holds your account, sign-in methods and two-factor sign-in
(without the authenticator app's secret or the recovery codes), passkeys,
devices and tokens, Claude connections (without their secrets),
notification settings and browsers, addresses, workspaces, and from the
workspaces open to you the projects you are a member of, the sessions you
started with their transcripts and the sessions you follow, and your
activity log. Deleting your account deletes all of that and the
workspaces only you are in; sessions you started in shared workspaces stay
there, credited to a deleted user, and the workspaces you are the primary
admin of pass to their longest-standing admin. Settings in the web app has
both, the deletion in its danger zone.

## Updating

```sh
fctl self update             # to the latest release
fctl self update --check     # is there one?
fctl self update --version 0.1.2
```

It downloads the release for this machine, checks its SHA-256, replaces
`fctl` (and the Linux agent next to it), restarts a host service on the new
version (sessions keep running) and refreshes installed completions.
`FCTL_DOWNLOAD` points it at another release server.

Installed with Homebrew (`brew install hyperstructure/tap/fctl`)? Update with
`brew upgrade fctl` instead; Homebrew owns those files.

The web app's Agent Floor says when a runner of yours is online with an older
fctl than the control plane, and shows this command.

## Shell completions

The installer sets up tab completion for your shell. To do it yourself:

```sh
fctl completions --install           # zsh, bash or fish, from $SHELL
fctl completions zsh > ~/.zfunc/_fctl  # or print them: bash, zsh, fish, elvish, powershell
```

`--install` writes the script under `~/.local/share/fctl/completions` (fish:
`~/.config/fish/completions`) and adds a marked block once to the end of
`~/.zshrc` or `~/.bashrc`. Bash on macOS starts as a login shell, which reads
only the first of `~/.bash_profile`, `~/.bash_login` and `~/.profile` that
exists, so the block goes in that file (a new `~/.profile` if there is none).
Delete the script and the block to undo it.

## Exit codes and output

Commands exit non-zero on failure and print errors to stderr. With `--json`,
results go to stdout as one JSON document (or one per line for streams such as
`fctl logs -f --json`), using the same shapes as the HTTP API.
