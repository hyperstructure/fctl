# What fills the cast's pages

The sessions, reviews, memory notes, retrospectives, schedules, tickets and
secrets of the cast in [../SKILL.md](../SKILL.md). Times are from the moment
of the shots, a Tuesday at 09:10 in Stockholm. `shop#231` is a session,
numbered in its project; `shop#1291` in a pull request's line is the pull
request in its repository (`romanescohq/shop`).

## Sessions

Each project's Sessions tab, newest first. Review sessions take numbers too
but stay off the tab: they are under [Reviews](#reviews). Older sessions
follow the ones here. Titles and descriptions are the agents' own, as `fctl
edit` sets them; a scheduled session's row also names its schedule.

### Shop

| # | Title | Started by | At 09:10 | Pull request | Description |
|---|---|---|---|---|---|
| 231 | Add Danish to the shop | Tomás, 08:52, from SHOP-419 | Running: watching CI | shop#1291, open, checks running | Translates the shop for the Copenhagen launch: Lucía's copy from SHOP-419, prices in kroner and delivery days from the Copenhagen depot. |
| 230 | Match the box builder to the new illustrations | Lucía, 08:15, two mockups attached | Paused: Lucía's plan is at its five-hour limit, resumes at 10:00 | | Restyles the box builder's cards around the new vegetable illustrations, from the two mockups. |
| 227 | Update 9 npm dependencies | Nightly dependency updates, for Tomás, 02:00 | Stopped | shop#1288, open, 23 checks passed | Updates the npm dependencies that have patch and minor releases in one pull request, and lists the two majors it left alone. |
| 224 | Let subscribers skip a week from the reminder email | Noor, yesterday 22:40 | Stopped | shop#1287, merged | Thursday's reminder email gets a link that skips next week's delivery in one click, with an undo on the page it opens. |
| 221 | Fix the delivery-day picker on Safari | Tomás, yesterday 16:20 | Stopped | shop#1286, merged | The picker lost its selection on Safari when the keyboard closed. It keeps it now, with a Playwright test in WebKit. |
| 219 | Show which farm each vegetable comes from | Jonas, yesterday 11:02, from SHOP-405 | Stopped | shop#1285, merged | Each item in a box links to the farm it comes from, with the farm's photo and a line about it. |
| 217 | Add a test for the postcode lookup | Sanna, Monday 14:05 | Stopped | shop#1283, merged | Covers the postcode lookup with tests, the 14 postcodes that border two delivery zones included. |
| 215 | Explain why a delivery day is full | Lucía, last Thursday | Stopped | shop#1281, merged | A full delivery day says why (the van is full, or the farm doesn't pick that day) and offers the nearest free one. |
| 212 | Explore gift boxes for companies | Jonas, last Wednesday, from SHOP-398 | Stopped | | An exploration: how companies could send boxes to their staff. The plan is in SHOP-398, waiting for Jonas to go ahead. |

### Platform

| # | Title | Started by | At 09:10 | Pull request | Description |
|---|---|---|---|---|---|
| 445 | Retry Stripe webhooks that time out | Asha, 08:31 | Waiting for a reply | platform#845, open, 14 checks passed | Webhooks whose processing fails are retried with backoff, and kept in a dead-letter table after five tries. The agent asks whether a dead letter should page someone. |
| 442 | Plan routes with milkfloat's time windows | Noor, yesterday 21:58 | Stopped | platform#843, open, 14 checks passed | Route planning moves from the nearest-neighbour heuristic to milkfloat, with each stop's delivery window. Driver breaks wait for milkfloat#311. |
| 440 | Split VAT on boxes with flowers | Asha, yesterday 13:30 | Stopped | platform#841, merged | Boxes with flowers carry two VAT rates on the invoice: food's for the vegetables and 25% for the flowers. |
| 437 | Find why two customers were charged twice | Asha, yesterday 09:12, from PLAT-238 | Stopped | platform#840, open | Two customers were charged twice last Wednesday when a webhook retry raced the invoice job. Charges are now idempotent per subscription and period. |
| 435 | Fix the invoice test that fails at midnight UTC | Weekly flaky test hunt, for Asha, Saturday 03:00 | Stopped | platform#836, merged | Three tests failed and then passed on a retry last week. Fixed the flakiest, which raced the clock at midnight UTC. |
| 433 | Speed up the distance matrix for Copenhagen | Noor, last Wednesday | Stopped | platform#833, merged | Builds the distance matrix for Copenhagen's 1,800 addresses in parallel and caches it for the day: planning takes 9 s instead of 70. |
| 430 | Cache delivery slots per postcode | Asha, last Tuesday | Stopped | platform#831, merged | Caches delivery slots per postcode for ten minutes, which takes the shop's slot query from 420 ms to 30 ms at the 95th percentile. |

### Driver app

| # | Title | Started by | At 09:10 | Pull request | Description |
|---|---|---|---|---|---|
| 77 | Show the delivery window on each stop | Aiko, 08:47 | Running | | Each stop in the route list shows the customer's delivery window, and turns amber when the van runs late for it. |
| 76 | Upgrade the map SDK | Aiko, yesterday 18:05 | Failed: the setup script's `npm ci` timed out reaching the registry | | Moves the app to the map SDK's next major version. |
| 75 | Show the next stop on the lock screen | Aiko, last Friday, from DEL-97 | Stopped | driver-app#417, merged | A Live Activity shows the next stop's address and delivery window on the lock screen, and moves on as stops are done. |
| 74 | Take a photo at the door when nobody answers | Aiko, last Wednesday | Stopped | driver-app#415, merged | When nobody answers, the driver photographs the box at the door, and the photo goes in the customer's delivery email. |

### Infra

| # | Title | Started by | At 09:10 | Pull request | Description |
|---|---|---|---|---|---|
| 38 | Check production for Terraform drift | Terraform drift check, for Kofi, 09:00 | Running | | Runs terraform plan for production and staging, and opens an issue in romanescohq/infra for anything that drifted. |
| 37 | Move staging to Postgres 18 | Kofi, yesterday 15:40 | Stopped | infra#211, draft | Prepares staging's move to Postgres 18 with a blue-green switch. Blocked: the agent's AWS role can't create the new parameter group. |
| 36 | Alert when webhook retries pile up | Kofi, last Thursday | Stopped | infra#209, merged | Adds an alarm on the webhook retry queue, and a runbook for it in docs/runbooks. |

### milkfloat, Benchmarks

| # | Title | Started by | At 09:10 | Pull request | Description |
|---|---|---|---|---|---|
| milkfloat#96 | Finish driver breaks from Anouk's pull request | Noor, 08:58 | Running | milkfloat#313, open, builds on #311 | Takes Anouk's driver breaks from milkfloat#311, her commits included, and adds the tests and fixes its review asked for. |
| milkfloat#92 | Publish wheels for free-threaded Python 3.14 | Noor, last Thursday | Stopped | milkfloat#310, merged | Builds `cp314t` wheels for Linux, macOS and Windows with maturin, beside the abi3 ones, and publishes them from the release workflow. |
| milkfloat#89 | Explain time windows in the docs | Lena, last Monday | Stopped | milkfloat#308, merged | A docs page on time windows, worked through with a bakery's morning round. |
| milkfloat-bench#41 | Benchmark main on 356 instances | Nightly benchmarks, for Noor, 01:00 | Stopped | bench#88, open | Ran the 356 Solomon and Gehring–Homberger instances against main: two got better, none worse. |

### punschrulle, Crimp Collective

| # | Title | Started by | At 09:10 | Pull request | Description |
|---|---|---|---|---|---|
| sourdough#12 | Remind me to feed the starter at 21:00 | Noor, yesterday 21:14 | Stopped | sourdough#31, merged | A push reminder at 21:00 on the evening before a bake, which the app on the home screen shows. |
| plantwatch#23 | Read the new soil sensor over I²C | Noor, Sunday | Stopped | plantwatch#19, merged | Reads the capacitive soil sensor over I²C rather than the analog pin, whose readings drifted as the board warmed up. |
| website#9 | Turn my meetup talk into a post | Noor, last Saturday | Stopped | noorhaddad.se#14, open | Turns the slides and notes of "Agents on the night shift" into a post, with the diagrams redrawn in SVG. |
| bookings#17 | Close bookings while the wall is reset | Elsa, Saturday | Stopped | bookings#52, merged | Admins mark a reset on the calendar; sessions it overlaps can't be booked, and members who booked get an email. |
| bookings#16 | Upgrade to Django 5.2 | Noor, two weeks ago | Stopped | bookings#51, merged | Moves the app from Django 4.2 to 5.2, both long-term releases, and fixes the three deprecations it hit. |

### The Agent Floor at 09:10

- Working now: shop#231, driver-app#77, infra#38 and milkfloat#96 at work;
  platform#445 waiting for Asha's reply; shop#230 paused until 10:00.
- Recently, the night's work: shop#227 (02:00), milkfloat-bench#41
  (01:00), platform#442 (from 21:58), shop#224 (from 22:40), sourdough#12
  (21:14); then yesterday's, driver-app#76 among them, failed.

### Who is watching

On shop#231's page at 09:10: Tomás, Lucía and Noor. Noor follows every
session she starts, and reads platform#442 in the Claude app on her phone
(Remote Control).

## Reviews

Review agents run on Asha's subscription in Shop and Platform, with Sonnet
at high effort; Dependabot's pull requests start reviews by themselves. In
milkfloat they run on Noor's, and Renovate's pull requests start them by
themselves; outside contributors' wait until a maintainer asks.

Each project's Reviews tab, newest first:

| Session | Pull request | Author | Commit | Result | Took | When |
|---|---|---|---|---|---|---|
| shop#232 | Add Danish to the shop, shop#1291 | `fctl-sh[bot]` | `a3f9c12` | Reviewing | | 09:08 |
| shop#229 | Bump zod from 4.1.12 to 4.1.13, shop#1289 | `dependabot[bot]` | `91d2e6b` | No issues | 19 s | 04:12 |
| shop#228 | Update 9 npm dependencies, shop#1288 | `fctl-sh[bot]` | `7be04d1` | No issues | 48 s | 02:41 |
| shop#226 | Let subscribers skip a week from the reminder email, shop#1287 | `fctl-sh[bot]` | `4e1c9a2` | No issues · 1 fixed | 1m 12s | 23:20 |
| shop#225 | Let subscribers skip a week from the reminder email, shop#1287 | `fctl-sh[bot]` | `0b77e31` | 1 issue | 1m 31s | 23:02 |
| shop#223 | Fix the delivery-day picker on Safari, shop#1286 | `fctl-sh[bot]` | `2c8e5f0` | No issues · 2 fixed | 1m 5s | yesterday 17:31 |
| shop#222 | Fix the delivery-day picker on Safari, shop#1286 | `fctl-sh[bot]` | `8a1f0c4` | 2 issues | 1m 40s | yesterday 16:58 |
| shop#220 | Show which farm each vegetable comes from, shop#1285 | `fctl-sh[bot]` | `c4d18e9` | No issues | 52 s | yesterday 11:40 |
| shop#218 | Add a test for the postcode lookup, shop#1283 | `fctl-sh[bot]` | `5e2b7d0` | No issues | 37 s | Monday 14:40 |
| shop#216 | Explain why a delivery day is full, shop#1281 | `fctl-sh[bot]` | `e09c3f4` | No issues | 1m 14s | last Thursday |
| platform#446 | Retry Stripe webhooks that time out, platform#845 | `fctl-sh[bot]` | `3d5a9f1` | No issues | 1m 22s | 08:58 |
| platform#444 | Plan routes with milkfloat's time windows, platform#843 | `fctl-sh[bot]` | `5d07aa4` | No issues · 2 fixed | 1m 51s | 00:37 |
| platform#443 | Plan routes with milkfloat's time windows, platform#843 | `fctl-sh[bot]` | `19c6e2d` | 2 issues | 2m 14s | yesterday 23:48 |
| platform#441 | Split VAT on boxes with flowers, platform#841 | `fctl-sh[bot]` | `6f20b8e` | No issues | 1m 47s | yesterday 14:10 |
| platform#439 | Make charges idempotent per subscription and period, platform#840 | `fctl-sh[bot]` | `9c4b2aa` | No issues · 1 fixed | 58 s | yesterday 11:05 |
| platform#438 | Make charges idempotent per subscription and period, platform#840 | `fctl-sh[bot]` | `e81b3d7` | 1 issue | 2m 5s | yesterday 10:20 |
| platform#436 | Fix the invoice test that fails at midnight UTC, platform#836 | `fctl-sh[bot]` | `b4f81c2` | No issues | 44 s | Saturday 03:41 |
| platform#434 | Speed up the distance matrix for Copenhagen, platform#833 | `fctl-sh[bot]` | `3a9de57` | No issues | 1m 30s | last Wednesday |
| platform#432 | Cache delivery slots per postcode, platform#831 | `fctl-sh[bot]` | `7a6e1d3` | No issues | 1m 9s | last Tuesday |
| platform#431 | Cache delivery slots per postcode, platform#831 | `fctl-sh[bot]` | `0f4c2b9` | Superseded by the next commit | | last Tuesday |
| milkfloat#95 | Update Rust crate pyo3 to v0.27.2, milkfloat#312 | `renovate[bot]` | `44c0f9e` | No issues | 22 s | 03:05 |
| milkfloat#94 | Add driver breaks, milkfloat#311 | `anoukvdberg` | `b2e7c40` | 3 issues | 2m 38s | yesterday 19:30, as Noor asked |
| milkfloat#93 | Publish wheels for free-threaded Python 3.14, milkfloat#310 | `fctl-sh[bot]` | `d61a0f7` | No issues | 1m 2s | last Thursday |
| milkfloat#91 | Fix a panic when a depot has no vehicles, milkfloat#309 | `o-adeyemi` | `7d9e3a1` | No issues | 41 s | last Tuesday, as Asha asked |
| milkfloat#90 | Explain time windows in the docs, milkfloat#308 | `fctl-sh[bot]` | `c27f9b8` | No issues | 33 s | last Monday |

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
  CONFLICT DO NOTHING`) and call Stripe only when the insert wins.
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
- **Session #224, yesterday.** Email templates are MJML in `emails/`, and
  `npm run email:dev` previews them at localhost:3030. Put a screenshot of a
  changed email in the pull request: Lucía reviews copy from those.
- **Session #224, yesterday.** Stripe Checkout runs in test mode with the
  project's `STRIPE_SECRET_KEY`. The card 4000 0025 0000 3155 asks for 3-D
  Secure, which the subscription flow has to get through.
- **Session #221, yesterday.** The delivery-day picker sets
  `inputMode="none"` on purpose: without it, Safari on iOS drops the
  selection when the keyboard closes.
- **Session #217, Monday.** The e2e tests (`npm run e2e`) need the
  platform's API: run `docker compose -f
  ~/repos/romanescohq/platform/compose.yaml up -d api` first, or every
  checkout test fails at the delivery-day step.
- **Lucía, last week.** Copy is written in Swedish first, in
  `messages/sv.json`; `en.json` and `da.json` follow, and `npm run
  i18n:check` fails on a key missing from any of them. Ask Lucía before
  changing words customers read.

### Platform

- **Session #445, today.** Answer Stripe's webhooks at once and do the work
  in the job queue (`jobs::enqueue`): a handler slower than Stripe's timeout
  gets the event again, which is how #437's double charges started.
- **Session #442, yesterday.** milkfloat's time windows are seconds after
  midnight in the depot's time zone, not UTC: convert with
  `Depot::local_seconds`, or every route plans an hour off in winter and two
  in summer.
- **Session #437, yesterday.** The invoice job takes a Postgres advisory
  lock (`pg_try_advisory_lock(4242)`), so tests that run it need a database
  each (`TestDb::isolated()`): on a shared one they wait for each other and
  time out after 30 s.
- **Session #437, yesterday.** Billing races show only with two workers:
  reproduce them with `scripts/replay-webhooks.sh --workers 2` against a
  copy of the day's webhook log.
- **Session #430, last week.** sqlx checks queries at build time against
  `.sqlx/`: after changing a query, run `cargo sqlx prepare --workspace`
  with the test database up (`docker compose up -d postgres`), or CI fails
  with "query data is out of date".
- **Asha, two weeks ago.** Billing changes need Asha's review on GitHub as
  well as the review agent's: ask for it in the pull request.
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
- **Infra, session #37.** Creating RDS parameter groups needs the
  infra-admin role, which agents don't have: ask Kofi in the pull request.
- **Infra, Kofi.** Sessions plan, people apply: the project's AWS keys are
  read-only, so `terraform apply` fails by design. The apply workflow
  applies after a merge.
- **milkfloat, session #96, today.** Breaks are stops without a place: code
  that walks a route's stops skips `Stop::Break` where it looks up
  coordinates (`route::geometry`, `export::geojson`).
- **milkfloat, session #92.** Build the Python bindings with `maturin
  develop --release` before `uv run pytest tests/python`, or pytest imports
  the last wheel you built.
- **milkfloat, session #89.** A change that improves a benchmark instance
  updates `data/bks.csv` (best known solutions) in the same pull request, or
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
- Challenges: Every route came out an hour off until it turned out
  milkfloat wants windows in the depot's local time, not UTC.
- Insights: Driver breaks aren't planned yet; they come with milkfloat#311.

**Platform #437, Asha, completed.**
- Worked on: Found why two customers were charged twice last Wednesday,
  and made charges idempotent per subscription and period (platform#840).
  The refunds are Jonas's to make.
- Approach: Replayed the day's webhook log against a copy of the database:
  a Stripe retry arrived while the invoice job held its lock, and both
  charged.
- Challenges: The review found that the first fix still raced; the
  idempotency key now goes in before the call to Stripe.
- Insights: Billing races need two workers to show:
  `scripts/replay-webhooks.sh --workers 2`.

**Shop #227, Tomás's nightly dependency updates, completed.**
- Worked on: Updated 9 dependencies with patch and minor releases
  (shop#1288).
- Challenges: Left two majors for a person, the Sentry SDK and ESLint:
  both need code changes, listed in the pull request.

**Infra #37, Kofi, blocked.**
- Worked on: Prepared staging's move to Postgres 18: the new instance, its
  parameter group and a blue-green switch (infra#211, a draft).
- Challenges: The agent's AWS role can't create RDS parameter groups, so
  the plan fails at the first new resource.
- Insights: Someone with the infra-admin role has to create
  `romanesco-pg18` by hand, or give the agents' role
  `rds:CreateDBParameterGroup` in staging.

**Driver app #75, Aiko, completed.**
- Worked on: A Live Activity with the next stop's address and delivery
  window on the lock screen, which moves on as stops are done
  (driver-app#417).
- Approach: A config plugin rather than ejecting, so the app keeps Expo's
  managed workflow.
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
| Shop | Nightly dependency updates | `0 2 * * 1-5`, 02:00 on weekdays | Tomás | Update the shop's npm dependencies that have patch or minor releases, run the unit and e2e tests, and open one pull request with all of them. Leave majors out and list them in its description. | Today 02:00: started shop#227 |
| Platform | Weekly flaky test hunt | `0 3 * * 6`, 03:00 on Saturdays | Asha | Look at the last week of CI runs on main, find the tests that failed and then passed on a retry, and fix the flakiest. Say in the pull request how often it failed. | Saturday: started platform#435 |
| Platform | Monday release notes | `30 7 * * 1`, 07:30 on Mondays | Jonas | Write last week's release notes from the pull requests merged in the shop, the platform and the driver app, in Swedish and English for the support team, and open them as a pull request to `docs/changelog`. | Monday: skipped, Jonas's plan was at its weekly limit |
| Infra | Terraform drift check | `0 9 * * 1-5`, 09:00 on weekdays | Kofi | Run terraform plan for production and staging. If anything drifted, open an issue in romanescohq/infra with what changed and the commit that last touched it; otherwise end without one. | Today 09:00: started infra#38, running |
| Benchmarks | Nightly benchmarks | `0 1 * * *`, 01:00 | Noor | Run the Solomon and Gehring–Homberger benchmarks against main and open a pull request to milkfloat/bench with the results, naming every instance that got worse. | Today 01:00: started milkfloat-bench#41 |
| Website | Monthly link check | `0 8 1 * *`, paused | Noor | Check every link on the site and fix or remove the broken ones. | Paused by Noor |

## Tickets

Romanesco's Linear workspace is connected; agents act as `fctl-sh` there.

| Project | Tracker | Settings |
|---|---|---|
| Shop | Linear, team Shop | Every issue of Shop; tickets start as their sessions start; label `explore` asks for an exploration |
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
| SHOP-398 | Gift boxes for companies (`explore`) | Todo, a plan posted | Jonas | shop#212 |
| PLAT-238 | Two customers charged twice | In Review | Asha | platform#437 |
| DEL-97 | Next stop on the lock screen | Done | Aiko | driver-app#75 |

## Secrets

Names only, set by the projects' owners; nobody shows a value.

- Shop: `STRIPE_SECRET_KEY`, `STRIPE_WEBHOOK_SECRET`, `RESEND_API_KEY`,
  `SENTRY_AUTH_TOKEN`.
- Platform: `STRIPE_SECRET_KEY`, `MAPBOX_TOKEN`, `SENTRY_AUTH_TOKEN`.
- Driver app: `EXPO_TOKEN`, `MAPBOX_TOKEN`.
- Infra: `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` (read-only keys),
  `CLOUDFLARE_API_TOKEN`.
- Benchmarks: `CODSPEED_TOKEN`.

## Project members

In Romanesco everyone is an editor in every project (the base role), and
Noor and Kofi, its admins, own them all. Besides: Tomás owns Shop, where
Lucía is a maintainer; Asha owns Platform, Aiko Driver app and Kofi Infra.
In milkfloat the base role is maintainer. In Crimp Collective Elsa owns
Bookings and Noor edits it.

## Noor's settings

- Profile: Noor Haddad, avatar `https://avatars.githubusercontent.com/punschrulle`.
- Email addresses: `noor@romanesco.farm` (primary: Romanesco and milkfloat)
  and `noor@noorhaddad.se` (punschrulle and Crimp Collective); commits
  credit the first.
- Two-factor sign-in: an authenticator app, and passkeys named "MacBook
  Pro" and "iPhone".
- Claude: "Romanesco Max" (the default) and "Personal", a Pro plan, which
  her sessions in punschrulle and Crimp Collective run on.
- Access tokens: "MacBook CLI", "Raycast" (read only) and "Release notes
  action" (one project).
- Notifications: push to "iPhone" and "MacBook Pro" when a session she
  follows waits for her, pauses or fails.

## Transcripts

Session pages need transcripts written for them. Two, as they stand at
09:10:

**shop#231, Add Danish to the shop** (Opus, high effort; 38 tool calls, 9
files changed, +486 −21, 14 minutes of work; context 74K of 200K).

1. Tomás, 08:52: "Add Danish to the shop for the Copenhagen launch. Lucía's
   copy is in SHOP-419. Prices in DKK, and delivery days come from the
   Copenhagen depot, `cph-1`."
2. The agent reads the ticket (`fctl ticket view SHOP-419`), names the
   session (`fctl edit`), and looks around: `messages/sv.json`,
   `i18n/config.ts`, the middleware that picks the locale from the path,
   `lib/money.ts`, the delivery-day picker.
3. It says what it found: the shop has `sv` and `en`; prices go through
   `formatPrice`, which knows only SEK; `sto-1` is hard-coded in two places
   where the depot should follow the locale.
4. It adds `da`, writes `messages/da.json` from the ticket's copy, teaches
   `formatPrice` DKK and picks the depot by locale. `npm run i18n:check`:
   412 keys in sv, en and da. `npm test`: 186 passed. `npm run e2e --
   --project=webkit e2e/checkout.da.spec.ts`: 4 passed.
5. It adds the memory note on prices, commits "Add Danish translations and
   DKK prices", pushes, opens shop#1291 and publishes two screenshots into
   it (`fctl publish box-builder-da.png checkout-da.png`).
6. 09:10: `gh pr checks 1291 --watch`, with 11 of 23 checks done, while the
   review agent reviews `a3f9c12`.

**platform#445, Retry Stripe webhooks that time out** (Sonnet, high effort;
waiting for Asha).

1. Asha, 08:31: "When processing a Stripe webhook fails we drop it. Retry
   with backoff, and park it in a dead-letter table after five tries."
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
