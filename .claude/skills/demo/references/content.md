# What fills the cast's pages

The sessions, reviews, memory notes, retrospectives, schedules, tickets and
secrets of the cast in [../SKILL.md](../SKILL.md). Times are from the moment
of the shots, a Tuesday at 09:10 in Stockholm: yesterday is Monday, and
"last Friday" is four days ago. `shop#231` is a session, numbered in its
project; `shop#1291` in a pull request's column is the pull request in its
repository (`romanescohq/shop`).

## Sessions

Each project's Sessions tab, newest first; older sessions follow. Review
sessions take numbers too but stay off the tab: they are under
[Reviews](#reviews). Titles and descriptions are the agents' own, as `fctl
edit` sets them. A scheduled session's row has a repeat mark, with its
schedule's name on hover.

### Shop

| # | Title | Started by | At 09:10 | Pull request | Description |
|---|---|---|---|---|---|
| 231 | Add Danish to the shop | Tomás, delegating SHOP-419, 08:52 | Working: watching CI | shop#1291, open, checks running | Translates the shop for the Copenhagen launch: Lucía's copy from SHOP-419, prices in kroner and delivery days from the Copenhagen depot. |
| 230 | Match the box builder to the new illustrations | Lucía, 08:15, two mockups attached | Paused: Lucía's plan is at its five-hour limit until 13:15 | | Restyles the box builder's cards around the new vegetable illustrations, from the two mockups. |
| 227 | Update 9 npm dependencies | Weekly dependency updates, for Tomás, 02:00 | Stopped | shop#1288, open, 23 checks passed | Updates the npm dependencies that have patch and minor releases in one pull request, and lists the two majors it left alone. |
| 224 | Let subscribers skip a week from the reminder email | Noor, yesterday 22:40 | Stopped | shop#1287, merged | Thursday's reminder email gets a link that skips next week's delivery in one click, with an undo on the page it opens. |
| 221 | Fix the delivery-day picker on Safari | Tomás, yesterday 16:20 | Stopped | shop#1286, merged | The picker lost its selection on Safari on iOS when the keyboard closed. It keeps it now; Tomás tried it on his phone. |
| 219 | Show which farm each vegetable comes from | Jonas, delegating SHOP-405, yesterday 11:02 | Stopped | shop#1285, merged | Each item in a box links to the farm it comes from, with the farm's photo and a line about it. |
| 217 | Add a test for the postcode lookup | Sanna, last Friday 14:05 | Stopped | shop#1283, merged | Covers the postcode lookup with tests, the 14 postcodes that border two delivery zones included. |
| 215 | Explain why a delivery day is full | Lucía, last Thursday | Stopped | shop#1281, merged | A full delivery day says why (the van is full, or the farm doesn't pick that day) and offers the nearest free one. |
| 212 | Explore gift boxes for companies | Jonas, delegating SHOP-398, last Wednesday | Stopped | | An exploration: how companies could send boxes to their staff. The plan is in SHOP-398, waiting for Jonas to go ahead. |

### Platform

| # | Title | Started by | At 09:10 | Pull request | Description |
|---|---|---|---|---|---|
| 445 | Retry Stripe webhook processing that fails | Asha, 08:31 | Waiting for Asha | platform#845, open, 14 checks passed | Webhooks whose processing fails are retried with backoff, and kept in a dead-letter table after five tries. The agent asks whether a dead letter should page someone. |
| 442 | Plan routes with milkfloat's time windows | Noor, yesterday 21:58 | Stopped | platform#843, open, 14 checks passed | Route planning moves from the nearest-neighbour heuristic to milkfloat, with each stop's delivery window. Driver breaks wait for milkfloat#311. |
| 440 | Split VAT on boxes with flowers | Asha, yesterday 13:30 | Stopped | platform#841, merged | Boxes with flowers carry two VAT rates on the invoice: food's for the vegetables and 25% for the flowers. |
| 437 | Find why two customers were charged twice | Asha, delegating PLAT-238, yesterday 09:12 | Stopped | platform#840, open | Two customers were charged twice last Wednesday, when a webhook retry raced the invoice job. Charges are now idempotent per subscription and period. |
| 435 | Fix the invoice test that fails at midnight UTC | Weekly flaky test hunt, for Asha, Saturday 03:00 | Stopped | platform#836, merged | Three tests failed and then passed on a retry last week. Fixed the flakiest, which raced the clock at midnight UTC. |
| 433 | Speed up the distance matrix for Copenhagen | Noor, last Wednesday | Stopped | platform#833, merged | Builds the distance matrix for Copenhagen's 1,800 addresses in parallel and caches it for the day: planning takes 9 s instead of 70. |
| 430 | Cache delivery slots per postcode | Asha, last Tuesday | Stopped | platform#831, merged | Caches delivery slots per postcode for ten minutes, which takes the shop's slot query from 420 ms to 30 ms at the 95th percentile. |

### Driver app

| # | Title | Started by | At 09:10 | Pull request | Description |
|---|---|---|---|---|---|
| 77 | Show the delivery window on each stop | Aiko, 08:35 | Working | | Each stop in the route list shows the customer's delivery window, and turns amber when the van runs late for it. |
| 76 | Upgrade the map SDK | Aiko, yesterday 18:05 | Stopped | | Found that the map SDK's next major needs Expo's next SDK first, and stopped there: the upgrade waits for Expo's. |
| 75 | Show the next stop on the lock screen | Aiko, delegating DEL-97, last Friday | Stopped | driver-app#417, merged | A Live Activity shows the next stop's address and delivery window on the lock screen, and moves on as stops are done. |
| 74 | Take a photo at the door when nobody answers | Aiko, last Wednesday | Stopped | driver-app#415, merged | When nobody answers, the driver photographs the box at the door, and the photo goes in the customer's delivery email. |

### Infra

| # | Title | Started by | At 09:10 | Pull request | Description |
|---|---|---|---|---|---|
| 38 | Check production and staging for Terraform drift | Weekly drift check, for Kofi, 09:00 | Working | | Runs terraform plan for production and staging, and opens an issue in romanescohq/infra for anything that drifted. |
| 37 | Move staging to Postgres 18 | Kofi, yesterday 15:40 | Stopped | infra#211, draft | Prepares staging's move to Postgres 18 two ways, a blue-green switch or a dump and restore, and asks Kofi to choose. |
| 36 | Alert when the job queue backs up | Kofi, last Thursday | Stopped | infra#209, merged | Adds an alarm on the job queue's depth and oldest job, and a runbook for it in docs/runbooks. |

### milkfloat, Benchmarks

| # | Title | Started by | At 09:10 | Pull request | Description |
|---|---|---|---|---|---|
| milkfloat#96 | Finish driver breaks from Anouk's pull request | Noor, 08:58 | Working | milkfloat#313, open | Takes Anouk's driver breaks from milkfloat#311, her commits included, and adds the tests and fixes its review asked for. |
| milkfloat#92 | Publish wheels for free-threaded Python 3.14 | Noor, last Thursday | Stopped | milkfloat#310, merged | Builds `cp314t` wheels for Linux, macOS and Windows with maturin, beside the abi3 ones, as `kenji-sato-dev` asked in milkfloat#305. |
| milkfloat#89 | Explain time windows in the docs | Lena, last Wednesday | Stopped | milkfloat#308, merged | A docs page on time windows, worked through with a bakery's morning round. |
| milkfloat-bench#41 | Benchmark main on 356 instances | Nightly benchmarks, for Noor, 01:00 | Stopped | bench#88, open | Ran the 356 Solomon and Gehring–Homberger instances against main: two got better, none worse. |

### punschrulle, Crimp Collective

| # | Title | Started by | At 09:10 | Pull request | Description |
|---|---|---|---|---|---|
| sourdough#12 | Send a feeding reminder the evening before a bake | Noor, yesterday 21:14 | Stopped | sourdough#31, merged | A push reminder at 21:00 the evening before a bake, which the app on the home screen shows. |
| plantwatch#23 | Read the new soil sensor over I²C | Noor, Sunday | Stopped | plantwatch#19, merged | Reads the capacitive soil sensor over I²C rather than the analog pin, whose readings drifted as the board warmed up. |
| website#9 | Turn the night-shift talk into a post | Noor, Saturday | Stopped | noorhaddad.se#14, open | Turns the slides and notes of "Agents on the night shift" into a post, with the diagrams redrawn in SVG. |
| bookings#17 | Close bookings while the wall is reset | Elsa, Saturday | Stopped | bookings#52, merged | Admins mark a reset on the calendar; sessions it overlaps can't be booked, and members who booked get an email. |
| bookings#16 | Upgrade to Django 5.2 | Noor, last Wednesday | Stopped | bookings#51, merged | Moves the app from Django 4.2 to 5.2, both long-term releases, and fixes the three deprecations it hit. |

### The Agent Floor at 09:10

- Working now, newest first: infra#38, milkfloat#96, shop#231 and
  driver-app#77 at work; platform#445 waiting for Asha; shop#230 paused.
- Recently, newest first: shop#227 (02:00), milkfloat-bench#41 (01:00),
  shop#224 (22:40), platform#442 (21:58), sourdough#12 (21:14), then the
  rest of yesterday's.

### Who is watching

On shop#231's page at 09:10: Tomás, Lucía and Noor. Noor follows every
session she starts, and has milkfloat#96 open in the Claude app on her
phone: "Romanesco Max" is a Sign in with Claude login, so her sessions join
Remote Control.

## Reviews

Review agents run with Sonnet at high effort: in Shop on Tomás's
subscription and in Platform on Asha's, where Dependabot's pull requests
start reviews by themselves; in milkfloat on Noor's, where Renovate's do,
and outside contributors' wait until a maintainer asks (Asha asked for #91,
Noor for #94; the audit log says so).

Each project's Reviews tab, newest first:

| Session | Pull request | Author | Commit | Result | Took | When |
|---|---|---|---|---|---|---|
| shop#232 | Add Danish to the shop, shop#1291 | `fctl-sh[bot]` | `a3f9c12` | Reviewing | | 09:08 |
| shop#229 | Bump actions/checkout from 7.0.1 to 7.0.2, shop#1289 | `dependabot[bot]` | `91d2e6b` | No issues | 19 s | 04:12 |
| shop#228 | Update 9 npm dependencies, shop#1288 | `fctl-sh[bot]` | `7be04d1` | No issues | 48 s | 02:41 |
| shop#226 | Let subscribers skip a week from the reminder email, shop#1287 | `fctl-sh[bot]` | `4e1c9a2` | No issues · 1 fixed | 1m 12s | yesterday 23:20 |
| shop#225 | Let subscribers skip a week from the reminder email, shop#1287 | `fctl-sh[bot]` | `0b77e31` | 1 issue | 1m 31s | yesterday 23:02 |
| shop#223 | Fix the delivery-day picker on Safari, shop#1286 | `fctl-sh[bot]` | `2c8e5f0` | No issues · 2 fixed | 1m 5s | yesterday 17:31 |
| shop#222 | Fix the delivery-day picker on Safari, shop#1286 | `fctl-sh[bot]` | `8a1f0c4` | 2 issues | 1m 40s | yesterday 16:58 |
| shop#220 | Show which farm each vegetable comes from, shop#1285 | `fctl-sh[bot]` | `c4d18e9` | No issues | 52 s | yesterday 11:40 |
| shop#218 | Add a test for the postcode lookup, shop#1283 | `fctl-sh[bot]` | `5e2b7d0` | No issues | 37 s | last Friday 14:40 |
| shop#216 | Explain why a delivery day is full, shop#1281 | `fctl-sh[bot]` | `e09c3f4` | No issues | 1m 14s | last Thursday |
| platform#446 | Retry Stripe webhook processing that fails, platform#845 | `fctl-sh[bot]` | `3d5a9f1` | No issues | 1m 22s | 08:58 |
| platform#444 | Plan routes with milkfloat's time windows, platform#843 | `fctl-sh[bot]` | `5d07aa4` | No issues · 2 fixed | 1m 51s | 00:37 |
| platform#443 | Plan routes with milkfloat's time windows, platform#843 | `fctl-sh[bot]` | `19c6e2d` | 2 issues | 2m 14s | yesterday 23:48 |
| platform#441 | Split VAT on boxes with flowers, platform#841 | `fctl-sh[bot]` | `6f20b8e` | No issues | 1m 47s | yesterday 14:10 |
| platform#439 | Make charges idempotent per subscription and period, platform#840 | `fctl-sh[bot]` | `9c4b2aa` | No issues · 1 fixed | 58 s | yesterday 11:05 |
| platform#438 | Make charges idempotent per subscription and period, platform#840 | `fctl-sh[bot]` | `e81b3d7` | 1 issue | 2m 5s | yesterday 10:20 |
| platform#436 | Fix the invoice test that fails at midnight UTC, platform#836 | `fctl-sh[bot]` | `b4f81c2` | No issues | 44 s | Saturday 03:41 |
| platform#434 | Speed up the distance matrix for Copenhagen, platform#833 | `fctl-sh[bot]` | `3a9de57` | No issues | 1m 30s | last Wednesday |
| platform#432 | Cache delivery slots per postcode, platform#831 | `fctl-sh[bot]` | `7a6e1d3` | No issues | 1m 9s | last Tuesday |
| platform#431 | Cache delivery slots per postcode, platform#831 | `fctl-sh[bot]` | `0f4c2b9` | Superseded | | last Tuesday |
| milkfloat#97 | Add driver breaks, building on #311, milkfloat#313 | `fctl-sh[bot]` | `8e3c1d9` | Reviewing | | 09:09 |
| milkfloat#95 | Update Rust crate pyo3 to v0.28.3, milkfloat#312 | `renovate[bot]` | `44c0f9e` | No issues | 22 s | 03:05 |
| milkfloat#94 | Add driver breaks, milkfloat#311 | `anoukvdberg` | `b2e7c40` | 3 issues | 2m 38s | yesterday 19:30 |
| milkfloat#93 | Publish wheels for free-threaded Python 3.14, milkfloat#310 | `fctl-sh[bot]` | `d61a0f7` | No issues | 1m 2s | last Thursday |
| milkfloat#91 | Fix a panic when a depot has no vehicles, milkfloat#309 | `o-adeyemi` | `7d9e3a1` | No issues | 41 s | last Thursday |
| milkfloat#90 | Explain time windows in the docs, milkfloat#308 | `fctl-sh[bot]` | `c27f9b8` | No issues | 33 s | last Wednesday |

What the reviews found, as they posted it on GitHub:

- **shop#1287, medium, fixed by `4e1c9a2`** (`app/skip/[token]/route.ts`):
  The skip token isn't tied to a delivery, so last week's email skips this
  week's. Sign the delivery's date into the token and refuse it once that
  day has passed.
- **shop#1286, low, fixed** (`e2e/day-picker.spec.ts`): The test waits with
  `page.waitForTimeout(500)`. Wait for the picker's selected state instead,
  or it flakes on a slow runner.
- **shop#1286, low, fixed** (`components/DayPicker.tsx`): `inputMode="none"`
  needs a comment saying why, or the next person removes it.
- **platform#840, high, fixed by `9c4b2aa`** (`src/billing/charge.rs`):
  `charge_once` calls Stripe before it records the idempotency key, so two
  workers that race both charge the card. Insert the key first (`ON
  CONFLICT DO NOTHING`), call Stripe only when the insert wins, and pass the
  key to Stripe as its `Idempotency-Key`, so that a retried call can't
  charge twice either.
- **platform#843, medium, fixed** (`src/routes/plan.rs`): Time windows go to
  milkfloat in UTC seconds; it wants seconds after midnight in the depot's
  time zone.
- **platform#843, low, fixed** (`src/routes/plan.rs`): `ROUTES_PLANNER`
  falls back to the old planner on any value but `milkfloat`, so a typo
  switches planners silently. Refuse unknown values at startup.
- **milkfloat#311, medium** (`src/insert.rs`): A break whose window ends
  before the vehicle's shift starts makes `insert_break` loop forever.
  Return `Infeasible` instead.
- **milkfloat#311, low** (`python/milkfloat/__init__.pyi`): The stubs lack
  `Break`, so type checkers reject the new argument.
- **milkfloat#311, low** (`README.md`): The example still builds `Vehicle`
  without `breaks=`.

## Memory

Each project's Memory tab, newest first. A note says which session's agent
wrote it, or which person did.

### Shop

- **Session #231, today.** Prices are integers in öre and øre (1995 is
  19,95 kr) everywhere. Format them with `formatPrice(amount, locale,
  currency)` from `lib/money.ts`, never `toFixed`: it writes a decimal point
  where Swedish and Danish write a comma.
- **Session #231, today, in place of Lucía's note.** Copy is written in
  Swedish first, in `messages/sv.json`; `en.json` and `da.json` follow, and
  `npm run i18n:check` fails on a key missing from any of them. Ask Lucía
  before changing words customers read.
- **Session #224, yesterday.** Email templates are MJML in `emails/`, and
  `npm run email:dev` previews them at localhost:3030. Put a screenshot of a
  changed email in the pull request: Lucía reviews copy from those.
- **Session #221, yesterday.** The delivery-day picker sets
  `inputMode="none"` on purpose: without it, Safari on iOS drops the
  selection when the keyboard closes.
- **Session #217, last Friday.** The e2e tests (`npm run e2e`) need the
  platform's API on port 8080, which the setup script starts from
  `~/repos/romanescohq/platform` with `scripts/dev-api.sh`. If it stopped,
  run that again, or every checkout test fails at the delivery-day step.
- **Stale.** Lucía, last Wednesday: "Copy is written in Swedish first, in
  `messages/sv.json`, then `en.json`; `npm run i18n:check` fails on a key
  missing from either. Ask Lucía before changing words customers read."
  Replaced by session #231's note.

### Platform

- **Session #445, today.** Answer Stripe's webhooks at once and do the work
  in the job queue (`jobs::enqueue`): a handler slower than Stripe's timeout
  gets the event again, which is how #437's double charges started.
- **Session #442, yesterday.** milkfloat's time windows are seconds after
  midnight in the depot's time zone, not UTC: convert with
  `Depot::local_seconds`, or every route plans an hour off in winter and two
  in summer.
- **Session #437, yesterday.** The invoice job holds a Postgres advisory
  lock (`pg_advisory_lock(4242)`) while it runs, so tests that run it need a
  database each (`TestDb::isolated()`): on a shared one they wait for each
  other and time out after 30 s.
- **Session #437, yesterday.** Billing races show only with two workers:
  reproduce them with `scripts/replay-webhooks.sh --workers 2` against a
  copy of the day's webhook log.
- **Asha, last Wednesday.** Billing changes need Asha's review on GitHub as
  well as the review agent's: ask for it in the pull request.
- **Session #430, last Tuesday.** CI builds with `SQLX_OFFLINE=true`, from
  the query data in `.sqlx/`: after changing a query, run `cargo sqlx
  prepare --workspace` against the sandbox's Postgres (the setup script
  starts it), or CI fails with "`SQLX_OFFLINE=true` but there is no cached
  data for this query".
- **Stale.** "The shop's delivery-slot query takes 400 ms or more for
  postcodes with many farms: don't add joins to it." Marked stale by
  session #430: "Cached per postcode since platform#831."

### Driver app, Infra, milkfloat, sourdough

- **Driver app, session #75.** EAS builds don't run in sandboxes: check
  that the app bundles with `npx expo export --platform ios`, and leave
  builds to the eas-build workflow.
- **Driver app, session #74.** Never log a `Stop`: it carries the door
  code. Log `stop.toLogSafe()`; the `no-unsafe-stop-log` lint rule fails CI
  otherwise.
- **Infra, session #37, yesterday.** Staging's parameter group has
  `rds.logical_replication` off, and a blue-green switch needs it on, which
  takes a reboot: plan a switch a day ahead.
- **Infra, Kofi.** Sessions plan, people apply: the project's AWS keys are
  read-only, so run `terraform plan -lock=false`, and leave applying to the
  apply workflow after a merge.
- **milkfloat, session #96, today.** Breaks are stops without a place: code
  that walks a route's stops skips `Stop::Break` where it looks up
  coordinates (`route::geometry`, `export::geojson`).
- **milkfloat, session #92.** Build the Python bindings with `maturin
  develop --release` before `uv run pytest tests/python`, or pytest imports
  the last wheel you built.
- **milkfloat, Noor.** A change that beats milkfloat's best result on a
  benchmark instance updates `data/best.csv` in the same pull request, or
  the bench check fails.
- **sourdough, session #12.** iOS shows web push only for the app added to
  the home screen: test reminders there, not in a Safari tab.

## Retrospectives

Shop's tab counts 131 completed, 14 partial and 3 blocked; Platform's 229,
31 and 9.

**Shop #224, Noor, completed.**
- Worked on: A "Skip next week" link in Thursday's reminder email that
  skips the delivery in one click, with an undo on the page it opens
  (shop#1287).
- Approach: Signed links with the delivery's date in them rather than a
  sign-in, since most people read the email on their phone; the platform
  already had the skip endpoint the app uses.
- Challenges: The review found that the first token wasn't tied to a
  delivery, so an old email would skip the wrong week; the second commit
  fixed it. The email preview isn't in the README.
- Insights: `npm run email:dev` previews the emails at localhost:3030. Put
  a screenshot of a changed email in the pull request.

**Platform #442, Noor, partial.**
- Worked on: Moved route planning from the nearest-neighbour heuristic to
  milkfloat, with each stop's delivery window (platform#843).
- Approach: Kept the old planner behind `ROUTES_PLANNER=legacy` for a week,
  so the delivery team can compare the two on real days.
- Challenges: The review found that the time windows went to milkfloat in
  UTC; it wants the depot's local time, so every route came out an hour
  off.
- Insights: Driver breaks aren't planned yet; they come with milkfloat#311.

**Platform #437, Asha, completed.**
- Worked on: Found why two customers were charged twice last Wednesday,
  and made charges idempotent per subscription and period (platform#840).
  The refunds are Jonas's to make.
- Approach: Replayed the day's webhook log against a copy of the database:
  a Stripe retry arrived while the invoice job was charging, and both
  charged.
- Challenges: The review found that the first fix still raced; the
  idempotency key now goes in before the call to Stripe, and to Stripe with
  it.
- Insights: Billing races need two workers to show:
  `scripts/replay-webhooks.sh --workers 2`.

**Shop #227, Tomás's weekly dependency updates, completed.**
- Worked on: Updated 9 dependencies with patch and minor releases
  (shop#1288).
- Challenges: Left two majors for a person, the Sentry SDK and ESLint:
  both need code changes, listed in the pull request.

**Infra #37, Kofi, blocked.**
- Worked on: Both ways to move staging to Postgres 18, in infra#211 (a
  draft): an RDS blue-green deployment, or a dump and restore into a new
  instance.
- Approach: One variable picks the way, so `terraform plan` shows each
  before anyone chooses.
- Challenges: A blue-green switch needs logical replication, which
  staging's parameter group has off; turning it on takes a reboot. A dump
  and restore takes about 20 minutes of downtime.
- Insights: Kofi has to choose. Production will need the same choice, with
  more data.

**Driver app #75, Aiko, completed.**
- Worked on: A Live Activity with the next stop's address and delivery
  window on the lock screen, which moves on as stops are done
  (driver-app#417).
- Approach: A config plugin that adds the widget extension at prebuild, so
  the app keeps Expo's Continuous Native Generation.
- Challenges: No simulator in the sandbox: the activity's state has unit
  tests, and Aiko tried it on her phone.

**Shop #217, Sanna, completed.**
- Worked on: Tests for the postcode lookup, the 14 postcodes that border
  two delivery zones included (shop#1283).
- Approach: Table-driven, from `tests/fixtures/postcodes.json`, so more
  postcodes are more rows.
- Insights: The fixtures hold only Stockholm's postcodes: Copenhagen's need
  adding before the launch.

## Schedules

| Project | Schedule | When (Europe/Stockholm) | Owner | Asks the agent to | Latest run |
|---|---|---|---|---|---|
| Shop | Weekly dependency updates | `0 2 * * 2`, Tuesdays at 02:00 | Tomás | Update the shop's npm dependencies that have patch or minor releases, run the unit and e2e tests, and open one pull request with all of them. Leave majors out and list them in its description. | Today 02:00: started shop#227 |
| Shop | Design check | `0 9 * * 2`, Tuesdays at 09:00 | Lucía | Compare the shop's pages with the design system's components and file one ticket in Shop listing where they drifted, with a screenshot of each. | Today 09:00: skipped, Lucía's plan was at its limit |
| Platform | Weekly flaky test hunt | `0 3 * * 6`, Saturdays at 03:00 | Asha | Look at the last week of CI runs on main, find the tests that failed and then passed on a retry, and fix the flakiest. Say in the pull request how often it failed. | Saturday: started platform#435 |
| Infra | Weekly drift check | `0 9 * * 2`, Tuesdays at 09:00 | Kofi | Run `terraform plan -lock=false` for production and staging. If anything drifted, open an issue in romanescohq/infra with what changed and the commit that last touched it; otherwise end without one. | Today 09:00: started infra#38, working |
| Benchmarks | Nightly benchmarks | `0 1 * * *`, every night at 01:00 | Noor | Run the Solomon and Gehring–Homberger instances against main and open a pull request to milkfloat/bench with the results, naming every instance that got worse. | Today 01:00: started milkfloat-bench#41 |
| Website | Monthly link check | `0 8 1 * *`, paused | Noor | Check every link on the site, and fix or remove the broken ones. | Paused by Noor |

## Tickets

Romanesco's Linear workspace is connected, by Noor. Agents act as the GitHub
App does, `fctl-sh` on app.fctl.sh (the dev stack's app is `fctl`).

| Project | Tracker | Settings |
|---|---|---|
| Shop | Linear, team Shop | Every issue of Shop; tickets start as their sessions start; the label `explore` asks for an exploration |
| Platform | Linear, team Platform | Every issue of Platform |
| Driver app | Linear, team Delivery | Every issue of Delivery |
| Infra, milkfloat | GitHub Issues | |
| The rest | None | |

Linear's projects: "Copenhagen launch" (all three teams, due next month)
and "Christmas boxes" (Shop).

| Ticket | Title | State | Delegated by | Session |
|---|---|---|---|---|
| SHOP-419 | Danish translations for the shop | In Progress | Tomás | shop#231 |
| SHOP-405 | Show where each vegetable is grown | Done | Jonas | shop#219 |
| SHOP-398 | Gift boxes for companies (`explore`) | In Progress, a plan posted | Jonas | shop#212 |
| PLAT-238 | Two customers charged twice | In Review | Asha | platform#437 |
| DEL-97 | Next stop on the lock screen | Done | Aiko | driver-app#75 |

## Secrets

Names only, set by the projects' owners; nobody shows a value.

- Shop: `STRIPE_SECRET_KEY` (test mode), `RESEND_API_KEY`,
  `SENTRY_AUTH_TOKEN`.
- Platform: `STRIPE_SECRET_KEY`, `STRIPE_WEBHOOK_SECRET`, `MAPBOX_TOKEN`,
  `SENTRY_AUTH_TOKEN`.
- Driver app: `MAPBOX_DOWNLOADS_TOKEN`, `SENTRY_AUTH_TOKEN`.
- Infra: `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY` (read-only),
  `CLOUDFLARE_API_TOKEN` (read-only).

## Project members

In Romanesco everyone is an editor in every project (the base role), and
Noor and Kofi, its admins, own them all. Besides: Tomás owns Shop, where
Lucía is a maintainer; Asha owns Platform, Aiko Driver app and Kofi Infra.
In milkfloat the base role is maintainer. In Crimp Collective Elsa owns
Bookings and Noor edits it.

## Noor's settings

- Profile: Noor Haddad, avatar `https://avatars.githubusercontent.com/punschrulle`.
- Email addresses: `noor@romanesco.farm`, the primary one, which Romanesco
  and milkfloat know her by and whose commits credit it; and
  `noor@noorhaddad.se`, for punschrulle and Crimp Collective, whose commits
  credit `4198311+punschrulle@users.noreply.github.com`.
- Two-factor sign-in: an authenticator app, and passkeys named "MacBook
  Pro" and "iPhone".
- Claude: "Romanesco Max", the default, a Sign in with Claude login; and
  "Personal", a Pro plan, which her sessions in punschrulle and Crimp
  Collective run on.
- Access tokens: "fctl on Noors-MacBook-Pro", the CLI's sign-in, and
  "Raycast", read only.
- Notifications: push to "Safari on iOS" (the app on her home screen) and
  "Safari on macOS", when a session she follows waits for her, pauses or
  fails.

## Transcripts

Session pages need transcripts written for them. Two, as they stand at
09:10:

**shop#231, Add Danish to the shop** (Opus, high effort; 38 tool calls, 9
files changed, +486 −21, 14 minutes of work; context 74K of 200K).

1. 08:52, fctl's first message, as for every delegated ticket: "Tomás
   Ferreira delegated SHOP-419 to fctl in Linear", the ticket's link, and
   the ticket as Linear gives it to agents: "Danish translations for the
   shop. For the Copenhagen launch: the shop in Danish, prices in DKK,
   delivery days from the Copenhagen depot (`cph-1`). Lucía's copy is in
   the comments." The session starts with the ticket's title.
2. The agent reads the ticket's comments (`fctl ticket view SHOP-419`),
   names the session "Add Danish to the shop" (`fctl edit`), and looks
   around: `messages/sv.json`, `i18n/config.ts`, the middleware that picks
   the locale from the path, `lib/money.ts`, the delivery-day picker.
3. It says what it found: the shop has `sv` and `en`; prices go through
   `formatPrice`, which knows only SEK; `sto-1` is hard-coded in two places
   where the depot should follow the locale.
4. It adds `da`, writes `messages/da.json` from Lucía's copy, teaches
   `formatPrice` DKK and picks the depot by locale. `npm run i18n:check`:
   412 keys in sv, en and da. `npm test`: 186 passed. `npm run e2e --
   e2e/checkout.da.spec.ts`: 4 passed.
5. It adds its two memory notes, the second in place of Lucía's, commits
   "Add Danish translations and DKK prices", pushes, opens shop#1291 and
   publishes two screenshots into it (`fctl publish box-builder-da.png
   checkout-da.png`: the instance and Shop publish screenshots).
6. 09:10: `gh pr checks 1291 --watch`, with 11 of 23 checks done, while the
   review agent reviews `a3f9c12`.

**platform#445, Retry Stripe webhook processing that fails** (Sonnet, high
effort; waiting for Asha).

1. Asha, 08:31: "When processing a Stripe webhook fails, we drop it. Retry
   it with backoff, and park it in a dead-letter table after five tries."
2. The agent reads `src/webhooks/`, `src/jobs/` and the migrations; writes
   `migrations/0142_webhook_dead_letters.sql` and `src/webhooks/retry.rs`;
   runs `cargo sqlx prepare --workspace` and `cargo test -p platform
   webhooks` (23 passed); pushes and opens platform#845. CI passes and the
   review finds nothing.
3. Its last message: "Retries are in platform#845: five tries with backoff
   from 30 seconds to 40 minutes, then the event goes to
   `webhook_dead_letters` with its last error. CI passed and the review
   found nothing. Before I call it done: should a dead letter page whoever
   is on call, or is the daily summary in #platform enough?"
