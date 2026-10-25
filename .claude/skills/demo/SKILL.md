---
name: demo
description: The made-up world that shows fctl in use in screenshots and videos for fctl.sh, X, LinkedIn and talks. Romanesco, a farm-box startup, with its people, workspaces, projects, runners and GitHub and Linear accounts; the sessions, reviews, memory notes, retrospectives and schedules that make it look lived in; and the avatars it needs. Use when asked for marketing screenshots or videos, for realistic demo data in cargo xtask dev, or for made-up people, companies or projects, and before adding anyone or anything to the cast.
---

# The demo cast

Screenshots and videos that show fctl in use, on fctl.sh, X, LinkedIn or in
a talk, show one made-up world: Romanesco, a startup that delivers boxes of
vegetables from farms around Stockholm, the people who work there and the
projects they run agents in. The same names in every shot make the shots
add up to one story, and keep real people's data out of them.

This file is the cast: who and what exists.
[references/content.md](references/content.md) is what fills their pages:
sessions, reviews, memory notes, retrospectives, schedules, tickets and
secrets.

Tests and docs have a cast of their own ([below](#the-test-cast)), which
stays there.

## Rules

- Show nothing that isn't in the cast: no real people, companies,
  repositories or tokens, and not the dev stack's own Dev Owner,
  Development workspace or Playground project.
- Nobody notable has these names, and the GitHub logins and email domains
  belonged to nobody when the cast was made (2026-10-25). Check a new one
  before you add it, and search the web for a person or company of that
  name in the same line of work:

  ```sh
  gh api users/LOGIN --silent && echo taken || echo free
  curl -s -H 'accept: application/dns-json' \
    'https://cloudflare-dns.com/dns-query?name=DOMAIN&type=NS' | jq .Status   # 3: unregistered
  ```

- No famous people, living or dead. A name like Grace Hopper reads as a
  test fixture, and the names and faces of people who died less than 70
  years ago can still be protected in advertising in some places.
- Time is relative. Every shot is taken on a Tuesday at 09:10 in Stockholm
  (`Europe/Stockholm`): the night's work is done and the morning's is under
  way. Write "yesterday 22:40", never a date.
- Shots agree with each other: a project's session count, the sessions at
  work on a runner, the Agent Floor's threads, who is in which workspace.
- Copy follows AGENTS.md: sentence case, plain and specific. Session titles
  say what the work is, in at most 60 characters.
- Use the pronouns the tables give.

## The story

Noor Haddad and Jonas Weber started Romanesco three years ago. It delivers
boxes of vegetables, bread and eggs from 40 farms around Stockholm to about
9,000 subscribers, and opens in Copenhagen next month. Eight of its people
run agents on fctl: on a Mac Studio and a Mac mini in the Stockholm office,
two rented Linux servers and Noor's laptop. Noor starts the larger pieces of
work in the evening and reads the pull requests over breakfast. Jonas hands
tickets to fctl in Linear. Review agents review every pull request in the
shop and the platform, on Tomás's and Asha's Claude subscriptions.

Noor also maintains milkfloat, the open-source route planner Romanesco plans
its vans' rounds with, keeps a few projects of her own, and helps her
climbing club with its booking app. Those are the other three workspaces in
her sidebar.

## People

The person signed in is Noor. Each person's avatar in fctl is their GitHub
avatar ([Avatars](#avatars)).

| Person | Email | GitHub | Who | Avatar |
|---|---|---|---|---|
| Noor Haddad (she) | `noor@romanesco.farm`; `noor@noorhaddad.se` in punschrulle and Crimp Collective | `punschrulle` | Co-founder and CTO, in Stockholm. Starts most sessions, maintains milkfloat. | Photo in a café window, winter coat |
| Jonas Weber (he) | `jonas@romanesco.farm` | `jweber87` | Co-founder and CEO, in Berlin. Writes tickets in Linear and delegates some to fctl; now and then starts a session to change copy or prices. | Professional headshot, plain backdrop |
| Asha Patel (she) | `asha@romanesco.farm` | `asha-pg` | Staff engineer, in London: orders, billing and Postgres. Platform's review agents run on her subscription. Co-maintains milkfloat. | Photo on a meetup stage, mid-sentence |
| Tomás Ferreira (he) | `tomas@romanesco.farm` | `tferreira-pt` | Frontend engineer, in Lisbon: the shop. Shop's review agents run on his subscription; owns its weekly dependency updates. | Outdoors, sunglasses pushed up, sea behind |
| Aiko Tanaka (she) | `aiko@romanesco.farm` | `aikotnk` | Mobile engineer, in Copenhagen: the driver app and the launch's deliveries. | With a cargo bike |
| Kofi Mensah (he) | `kofi@romanesco.farm` | `kofiops` | Platform engineer, in Amsterdam: infrastructure, CI and the runners. Workspace admin. Keeps his Claude token on his runners. | Pixel art |
| Lucía Navarro (she) | `lucia@romanesco.farm` | `luciadraws` | Product designer, in Barcelona. Starts sessions with mockups attached; on the Pro plan, which pauses them now and then. | Illustrated self-portrait |
| Sanna Virtanen (they) | `sanna@romanesco.farm` | `sannavirtanen` | Engineer, in Helsinki, three weeks in: small first pull requests. | None yet: initials |
| Inês Carvalho (she) | `ines@romanesco.farm` | none yet | Data engineer, starts next Monday: her invitation waits. | None |
| Lena Fischer (she) | `lena@milkfloat.dev` | `lfischer-rs` | milkfloat maintainer, in Berlin: the Python bindings and the docs. | Photo with a bicycle helmet |
| Elsa Berglund (she) | `elsa@crimpcollective.org` | `elsaberglund` | Runs the Crimp Collective's volunteers; wrote its booking app years ago. | On a climbing wall, chalky hands |

On GitHub only, as authors of milkfloat's pull requests from outside: Anouk
van den Berg (`anoukvdberg`, she), Oluwaseun Adeyemi (`o-adeyemi`, he) and
Kenji Sato (`kenji-sato-dev`, he, who keeps GitHub's placeholder avatar).
Bots: `fctl-sh[bot]` (agents' pull requests), `dependabot[bot]` in
Romanesco's repositories, `renovate[bot]` in milkfloat's.

## Workspaces

In the sidebar in this order, with these projects.

| Workspace | Slug | Avatar | People | Settings |
|---|---|---|---|---|
| Romanesco | `romanesco` | Uploaded: the Romanesco logo | Noor (primary admin), Kofi (admin); Jonas, Asha, Tomás, Aiko, Lucía, Sanna; Inês invited | Base role editor; two-factor required; GitHub `romanescohq`; Linear connected |
| milkfloat | `milkfloat` | Uploaded: the milkfloat logo | Noor (primary admin), Asha (admin), Lena | Base role maintainer; two-factor required; GitHub `milkfloat` |
| punschrulle | `punschrulle` | Uploaded: Noor's avatar | Noor | Invites skipped; GitHub `punschrulle` |
| Crimp Collective | `crimp-collective` | None: CC on pink | Elsa (primary admin), Noor | Base role editor; GitHub `crimpcollective` |

## GitHub and Linear

| Account | Kind | Repositories | Avatar |
|---|---|---|---|
| `romanescohq` | GitHub organization | `shop`, `platform`, `driver-app`, `infra`, `design-system` | Romanesco logo |
| `milkfloat` | GitHub organization | `milkfloat`, `milkfloat.dev`, `bench` | milkfloat logo |
| `punschrulle` | GitHub user: Noor | `sourdough`, `plantwatch`, `noorhaddad.se` | Noor |
| `crimpcollective` | GitHub organization | `bookings` | Crimp Collective logo |
| Romanesco, `linear.app/romanesco` | Linear workspace, connected by Noor | Teams Shop (`SHOP`, green), Platform (`PLAT`, blue), Delivery (`DEL`, orange) | Romanesco logo |

## Projects

A project's count leaves out its review sessions, which take numbers
too: Shop's newest session is #231 of its 164.

| Project | Slug | Repositories (primary first) | Sessions | Description |
|---|---|---|---|---|
| Shop | `shop` | `romanescohq/shop`; `romanescohq/design-system` (write), `romanescohq/platform` (read) | 164, 2 active | 🥕 The web shop: build a box, pick a delivery day, pay and manage the subscription. Next.js, TypeScript and Stripe. |
| Platform | `platform` | `romanescohq/platform`; `romanescohq/infra` (read) | 287, 1 active | Orders, subscriptions, farms and delivery routes behind every app. Rust (axum, sqlx) on Postgres, with milkfloat planning the vans' routes. |
| Driver app | `driver-app` | `romanescohq/driver-app` | 77, 1 active | 📱 The app drivers deliver with: the day's route, door codes, a photo at the door when nobody answers. Expo and React Native. |
| Infra | `infra` | `romanescohq/infra` | 38, 1 active | Terraform for AWS and Cloudflare, and the workflows that deploy everything. Agents plan; people apply. |
| milkfloat | `milkfloat` | `milkfloat/milkfloat`; `milkfloat/milkfloat.dev` (write) | 61, 1 active | 🚐 A vehicle-routing solver in Rust with Python bindings: time windows, capacities, driver breaks, and re-planning while the vans are out. MIT licensed. |
| Benchmarks | `milkfloat-bench` | `milkfloat/bench`; `milkfloat/milkfloat` (read) | 41 | Nightly runs of the Solomon and Gehring–Homberger instances against main, published at milkfloat.dev/bench. |
| sourdough | `sourdough` | `punschrulle/sourdough` | 12 | 🍞 Feeding reminders for my starter, and a bake-day plan that works back from when the bread should be ready. SvelteKit, installed on my phone. |
| plantwatch | `plantwatch` | `punschrulle/plantwatch` | 23 | 🌱 Soil sensors on ESP32s in every pot, Rust firmware on embassy, and a dashboard that says which plant is thirsty. |
| Website | `website` | `punschrulle/noorhaddad.se` | 9 | noorhaddad.se: notes on building Romanesco, recipes and the odd talk. Astro on Cloudflare Pages. |
| Bookings | `bookings` | `crimpcollective/bookings` | 17 | Session bookings, memberships and the route board for the club's wall. Django, kept alive by volunteers. |

Every default branch is `main`. A project's repositories all belong to one
GitHub account, so Platform uses milkfloat as a crate, not as a repository.
Review agents are on in Shop (on Tomás's subscription), Platform (on
Asha's) and milkfloat (on Noor's).

## Runners

Romanesco names its machines after vegetables. What each is doing at 09:10:

| Runner | Machine | os/arch | CPUs, memory | Driver | Capacity | Registered by | Runs sessions of | At 09:10 |
|---|---|---|---|---|---|---|---|---|
| `kohlrabi` | Mac Studio, Stockholm office | macos/aarch64 | 16, 128 GB | apple | 6 | Kofi | Romanesco | Online: shop#231, shop#230, platform#445 |
| `parsnip` | Mac mini, Stockholm office | macos/aarch64 | 14, 64 GB | apple | 4 | Kofi | Romanesco | Online: the review of shop#1291 |
| `celeriac` | Rented server, Helsinki | linux/x86_64 | 32, 128 GB | docker | 10 | Kofi | Romanesco | Online: infra#38 |
| `turnip` | Rented server, Falkenstein | linux/x86_64 | 16, 64 GB | docker | 6 | Kofi | Romanesco | Draining since 08:40, by Kofi: driver-app#77 left |
| `Noors-MacBook-Pro` | Noor's laptop | macos/aarch64 | 16, 48 GB | apple | 2 | Noor | Romanesco (Noor's own), milkfloat, punschrulle | Online: milkfloat#96, the review of milkfloat#313 |
| `fika` | Mini PC under Noor's TV | linux/x86_64 | 16, 32 GB | docker | 3 | Noor | punschrulle | Online, idle |
| `routeboard` | The club's old Mac mini behind the route-board screen | macos/aarch64 | 8, 16 GB | lima | 2 | Elsa | Crimp Collective | Offline, seen 2 days ago |

Kofi drains `turnip` for a kernel update; fctl shows when and by whom, not
why. Every host keeps itself up to date, and Kofi's runners keep his Claude
token. A runner runs a workspace's sessions only once the person who
registered it adds it there, as a member.

## Claude

| Person | Connections | At 09:10 |
|---|---|---|
| Noor | "Romanesco Max" (max, default, a Sign in with Claude login, so her sessions join Remote Control); "Personal" (pro) for punschrulle and Crimp Collective | Five hours 34% used, week 58% |
| Asha | Max; Platform's review agents run on it | Week 71% |
| Kofi | Max, kept on his runners | |
| Tomás | Max; Shop's review agents run on it | |
| Aiko | Max | |
| Jonas, Lucía, Sanna, Lena, Elsa | Pro | Lucía's five hours at the limit until 13:15: shop#230 paused, her design check skipped |

## Avatars

Square images, 512 × 512 px, in `assets/avatars/` under these names. fctl
draws people round, and workspaces and GitHub accounts as rounded squares,
so keep faces and logos clear of the corners. A workspace avatar can be at
most 256 KB. Mix styles as real people do: photos, an illustration, pixel
art.

| File | Shows | Where |
|---|---|---|
| `punschrulle.png` | Noor | Noor; the punschrulle workspace; GitHub `punschrulle` |
| `jweber87.png` | Jonas | Jonas |
| `asha-pg.png` | Asha | Asha |
| `tferreira-pt.png` | Tomás | Tomás |
| `aikotnk.png` | Aiko | Aiko |
| `kofiops.png` | Kofi | Kofi |
| `luciadraws.png` | Lucía | Lucía |
| `lfischer-rs.png` | Lena | Lena |
| `elsaberglund.png` | Elsa | Elsa |
| `anoukvdberg.png`, `o-adeyemi.png` | Anouk, Oluwaseun | Their comments on milkfloat's pull requests |
| `romanescohq.png` | A romanesco floret's spiral, one color on a solid ground | The Romanesco workspace; GitHub `romanescohq`; the Linear workspace |
| `milkfloat.png` | A small electric milk float, side on | The milkfloat workspace; GitHub `milkfloat` |
| `crimpcollective.png` | A chalky handprint | GitHub `crimpcollective` (the workspace keeps its initials) |

Sanna, Inês and Kenji have none, so their initials, or GitHub's placeholder,
show as they would for real people.

## Making the shots

The screenshots skill covers the mechanics: `cargo xtask dev`, Playwright
with reduced motion and a seeded `Math.random`, both themes, a phone width
and `fctl publish`. What the cast adds:

- **Loading it.** Nothing loads the cast yet. As its people, the CLI makes
  most of it: accounts by invitation from the dev owner, workspaces and
  their avatars (`fctl workspaces set --avatar`), projects, members,
  invitations, secrets, schedules and memory notes, and sessions run by
  fake-claude, which renames them, adds memory and records retrospectives
  on cue (docs/development.md). The dev stack has no GitHub App or Linear,
  so repositories, pull requests and their checks, reviews and the Linear
  connection go into local D1 by hand: the project's memory (`fctl memory`)
  has a note on each. Runners show the machine their host reports, so the
  table's seven take more than the one dev host. Sign in as Noor, not as
  the dev owner.
- **Avatars.** Give each person `https://avatars.githubusercontent.com/<login>`
  as their avatar, the address fctl also loads GitHub accounts' avatars
  from, and answer those requests from `assets/avatars` with Playwright's
  `page.route`. Without that, GitHub serves its grey placeholder for these
  logins, never a stranger's face. Leave the bots' logos to GitHub.
- **Transcripts.** fake-claude's sessions are short and generic, which
  shows on a session page. Shots of a session page need a transcript
  written for it; content.md outlines two.
- **The clock.** Times show relative to now ("16h", "6d"), dates from a
  week back, and the Agent Floor greets by the time of day. Set the
  browser's clock to a Tuesday 09:10 with Playwright's `page.clock` and its
  `timezoneId` to `Europe/Stockholm`, and load the cast with times relative
  to it.

Worth shooting: the Agent Floor at 09:10, Shop's Sessions tab, shop#231's
session page mid-turn, Platform's Reviews and Memory tabs, Romanesco's
Runners and People tabs, Shop's Tickets tab, and a session at a phone's
width.

## The test cast

Tests and docs use their own names, which read as examples there. Keep
them there, and out of marketing shots:

- People: Ada Lovelace (`ada@example.com`, GitHub `ada`), who is signed
  in; Grace Hopper (`grace@example.com`), who is invited, shared with and
  made admin; and walk-ons: Alan (`alan@example.com`, an early-access
  request), Maya (who invited someone), Bo Diddley (who resumed Ada's
  session), Cher (a name without a family name), Bob. cargo xtask dev signs
  in Dev Owner (`dev@fctl.test`).
- Workspaces and projects: Development and Playground (cargo xtask dev),
  Side project (`side-project`), Acme (`acme`), Ada's lab; `my-app` ("Web
  app", "The storefront"), `web`, `api`.
- GitHub `acme/web`, `acme/api`, `my-org/my-app`; Linear `ENG-1`.
- Runners: `dev-host`, `studio mac`, `spare mini`, `grace-mini`, `old-box`.
