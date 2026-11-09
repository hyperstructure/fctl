// Screenshots of the demo cast in `cargo xtask demo`'s stack, as Noor sees it.
//
//   cd ~/screenshots   # where Playwright is installed (the screenshots skill)
//   node <repo>/xtask/demo/shoot.mjs [--out DIR] [--only floor,session] [--theme dark|light] [--size 1440x900]
//   node <repo>/xtask/demo/shoot.mjs --page NAME=PATH   # more pages: {project#number} in PATH is that session's
//   node <repo>/xtask/demo/shoot.mjs --features DIR [--only sessions,review-agents/off]
//                                      # fctl.sh's feature pages' shots, as WebP in DIR/<page>/<name>-<theme>.webp
//   node <repo>/xtask/demo/shoot.mjs --open             # a browser to look around or record in, on your machine
//
// It signs in as Noor with the seed's sign-in of hers (her account has a
// passkey and an authenticator app), shows the cast's avatars where the app
// loads GitHub's (avatars.githubusercontent.com/<login>), answers what the
// stack can't ask GitHub or Linear (review agents' problem, Linear's teams,
// the GitHub App the instance lacks), and holds the browser's clock at the
// moment of the shots, 09:10 in the zone `cargo xtask demo` picked
// (.fctl-demo/shots.json), so that the app shows the story's times. Each shot
// is taken in both themes; the phone ones at 390 × 844, or with --features at
// 390 × 788, below the status bar that fctl.sh's phone frames draw.

import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, "../..");
// Playwright comes from where the script runs, not from the repository.
const { chromium } = createRequire(path.join(process.cwd(), "noop.js"))("playwright");

const args = process.argv.slice(2);
const opt = (name, fallback) => {
  const i = args.indexOf(name);
  return i >= 0 && args[i + 1] ? args[i + 1] : fallback;
};
const features = opt("--features", "");
const out = path.resolve(features || opt("--out", path.join(process.cwd(), "demo")));
const only = opt("--only", "")
  .split(",")
  .filter(Boolean);
const themes = opt("--theme", "dark,light").split(",");
const [width, height] = opt("--size", "1440x900").split("x").map(Number);
const open = args.includes("--open");
const manifest = JSON.parse(readFileSync(path.join(root, ".fctl-demo/shots.json"), "utf8"));
const avatars = path.join(here, "avatars");

const session = (ref) => `/s/${manifest.sessions[ref]}`;

/** Scrolls the page so that the first element with `text` sits `gap` pixels below the top. */
async function scrollTo(page, text, gap = 96) {
  const el = page.getByText(text, { exact: false }).first();
  await el.waitFor({ timeout: 10_000 });
  await el.evaluate((e, gap) => {
    const top = e.getBoundingClientRect().top + window.scrollY - gap;
    window.scrollTo({ top: Math.max(0, top), behavior: "instant" });
  }, gap);
  await page.waitForTimeout(400);
}

/** Scrolls the session page's details beside the transcript to the section with `text`. */
async function railTo(page, text) {
  const el = page.getByText(text, { exact: false }).first();
  await el.waitFor({ timeout: 10_000 });
  await el.evaluate((e) => e.scrollIntoView({ block: "center", behavior: "instant" }));
  await page.waitForTimeout(400);
}

/**
 * Gets a page ready to shoot: a passkey authenticator, as the people's
 * browsers have (headless Chromium has none, and the sign-in page would say
 * so), and app.fctl.sh's address where the app shows the stack's.
 */
async function ready(context, page) {
  const cdp = await context.newCDPSession(page);
  await cdp.send("WebAuthn.enable");
  await cdp.send("WebAuthn.addVirtualAuthenticator", {
    options: {
      protocol: "ctap2",
      transport: "internal",
      hasResidentKey: true,
      hasUserVerification: true,
      isUserVerified: true,
    },
  });
}

/** Shows app.fctl.sh's address where the page shows the stack's. */
async function dress(page) {
  await page.evaluate((app) => {
    const walk = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
    for (let n = walk.nextNode(); n; n = walk.nextNode()) {
      if (n.nodeValue?.includes(app)) n.nodeValue = n.nodeValue.replaceAll(app, "https://app.fctl.sh");
    }
  }, new URL(manifest.app).origin);
}

/**
 * Brings the open tab of a project's or workspace's tabs into view, as
 * someone would: on a phone they scroll sideways, and open at the first.
 */
async function showTab(page) {
  await page.evaluate(() => {
    for (const a of document.querySelectorAll('nav a[data-status="active"][href*="tab="]')) {
      const strip = a.closest("nav");
      if (!strip || strip.scrollWidth <= strip.clientWidth) continue;
      const tab = a.getBoundingClientRect();
      const view = strip.getBoundingClientRect();
      if (tab.left >= view.left && tab.right <= view.right) continue;
      strip.scrollLeft += tab.left + tab.width / 2 - (view.left + view.width / 2);
    }
  });
}

/**
 * Opens a changed file's diff over the session's page, as someone would
 * from its Changes: in the rail, or on a phone in the details sheet. Waits
 * for its colors, and moves the pointer off what it clicked, so that the
 * dialog's buttons under it don't show hovered.
 */
async function openDiff(page, file, { phone = false } = {}) {
  if (phone) await page.getByRole("button", { name: "Details" }).click();
  const name = new RegExp(`^${file.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}, `);
  await page.getByRole("button", { name }).first().click();
  await page.locator("dialog[open] .diff-code span[style]").first().waitFor({ timeout: 10_000 });
  await page.mouse.move(1, 1);
  await page.waitForTimeout(400);
}

/**
 * A card measured from what it shows: `x` and its size as given, in CSS pixels, and its top `dy`
 * from the first visible element `find` gives (a locator, or the text of one), kept in the window.
 */
const near = (find, [x, dy, width, height]) => async (page) => {
  const el = (typeof find === "string" ? page.getByText(find, { exact: false }) : find(page))
    .locator("visible=true")
    .first();
  const r = await el.boundingBox({ timeout: 10_000 });
  const y = Math.max(0, Math.min(r.y + dy, page.viewportSize().height - height));
  return [x, Math.round(y), width, height];
};

/** Opens the sidebar on a phone, as its menu button does. */
async function openMenu(page) {
  await page.getByRole("button", { name: "Open menu" }).first().click();
  await page.waitForTimeout(600);
}

/** Opens the transcript's setup row, and brings it up below the page's header, its log under it. */
async function openSetup(page) {
  await page.locator('button:has-text("Sandbox ready in")').first().click();
  await page.waitForTimeout(500);
  // Below the page's header, which the turn's bar makes two rows tall once the page scrolls.
  await scrollTo(page, "Sandbox ready in", 112);
}

/** Settings' new access token, filled in as Noor makes one for the team's chat bot. */
async function newToken(page) {
  await page.getByRole("button", { name: "New access token" }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByPlaceholder("PR reviews").fill("Team chat");
  await dialog.getByLabel("Run sessions").check();
  await dialog.getByLabel("Only some").check();
  await dialog.getByRole("checkbox", { name: "Shop" }).check();
  await dialog.getByRole("checkbox", { name: "Platform" }).check();
  await dialog.getByRole("button", { name: "Create token" }).scrollIntoViewIfNeeded();
  await page.waitForTimeout(300);
  return dialog;
}

// The shots of fctl.sh's feature pages (`--features`), by page: what to
// show and how to get there. `act` runs once the page has settled, and
// `size` gives a shot a window of its own size. `card` is the part of a
// shot that the features overview shows in its page's tile, [x, y, width,
// height] in CSS pixels (or measured from an element, `near`), which it
// writes as `<page>/card-<theme>.webp` at twice that size; a `cardOnly`
// shot is taken for its card alone.
const pages = {
  sessions: [
    { name: "floor", path: "/", settle: 3000 },
    { name: "session", path: session("shop#231"), settle: 2500 },
    {
      name: "follow-up",
      path: session("driver-app#75"),
      // Its failed check just under the page's header, and Lucía's comment and the answer below.
      act: (page) => scrollTo(page, "A check failed", 66),
    },
    { name: "phone-floor", path: "/", phone: true, settle: 3000 },
    // platform#445's agent asks whether a dead letter should page someone, with two answers to click.
    { name: "asking", path: session("platform#445"), settle: 2500 },
    { name: "phone-asking", path: session("platform#445"), phone: true, settle: 2500 },
    // What needs Noor, and her workspaces' projects, in the sidebar a phone opens.
    { name: "phone-needs", path: "/", phone: true, settle: 3000, act: openMenu },
    { name: "phone-session", path: session("shop#231"), phone: true, settle: 2500 },
    // Asha's agent's fix for the double charge, a file of it over the page.
    { name: "diff", path: session("platform#437"), act: (page) => openDiff(page, "src/billing/charge.rs") },
    // The shop's prices in Danish kroner, as shop#231 changed them.
    {
      name: "phone-diff",
      path: session("shop#231"),
      phone: true,
      settle: 2500,
      act: (page) => openDiff(page, "src/lib/money.ts", { phone: true }),
    },
  ],
  workspaces: [
    { name: "workspace", path: "/w/romanesco" },
    { name: "project", path: "/p/shop" },
    { name: "settings", path: "/p/shop?tab=settings" },
    { name: "people", path: "/w/romanesco?tab=people" },
    { name: "members", path: "/p/shop?tab=members" },
    { name: "workspace-settings", path: "/w/romanesco?tab=settings" },
    { name: "phone-workspace", path: "/w/romanesco", phone: true },
    { name: "phone-project", path: "/p/shop", phone: true },
    // The sidebar's workspaces and their projects' icons, in a window tall enough for all four.
    {
      name: "sidebar",
      path: "/",
      size: "1440x1400",
      settle: 3000,
      // From the first workspace down, below what needs Noor.
      card: near((page) => page.locator("aside").getByText("Romanesco", { exact: true }), [0, -14, 287, 800]),
      cardOnly: true,
    },
  ],
  sandboxes: [
    { name: "setup", path: session("shop#231"), act: openSetup, card: near("Sandbox ready in", [328, -14, 760, 390]) },
    { name: "build", path: session("milkfloat#89"), act: openSetup },
    {
      name: "settings",
      path: "/p/milkfloat?tab=sandbox",
      act: (page) => scrollTo(page, "Setup script", 180),
    },
    { name: "runners", path: "/w/romanesco?tab=runners" },
    { name: "phone-setup", path: session("shop#231"), phone: true, act: openSetup },
  ],
  linear: [
    {
      name: "delegated",
      path: session("shop#231"),
      act: (page) => page.evaluate(() => window.scrollTo(0, 0)),
    },
    // Tomás's delegation and the ticket, near the page's top, clear of what the agent does now.
    {
      name: "delegation",
      path: session("shop#231"),
      act: (page) => scrollTo(page, "delegated", 140),
      card: near("delegated", [329, -14, 620, 270]),
      cardOnly: true,
    },
    {
      name: "exploration",
      path: session("shop#212"),
      act: (page) => scrollTo(page, "Replied on SHOP-398", 220),
    },
    { name: "tickets", path: "/p/shop?tab=tickets" },
    { name: "workspace", path: "/w/romanesco?tab=linear" },
    // Romanesco's Linear on a phone, where its admin connected it.
    { name: "phone-linear", path: "/w/romanesco?tab=linear", phone: true },
    {
      name: "phone-delegated",
      path: session("driver-app#75"),
      phone: true,
      act: (page) => page.evaluate(() => window.scrollTo(0, 0)),
    },
  ],
  "review-agents": [
    { name: "reviews", path: "/p/platform?tab=reviews", card: [418, 506, 640, 390] },
    { name: "off", path: "/p/driver-app?tab=reviews" },
    { name: "review", path: session("platform#438"), settle: 2000 },
    {
      name: "findings",
      path: session("platform#437"),
      act: (page) => scrollTo(page, "Two workers can still both charge the card", 260),
    },
    { name: "outside", path: session("milkfloat#94"), settle: 2000 },
    // The shop's reviews, on Tomás's subscription: shop#1291's under way.
    { name: "shop-reviews", path: "/p/shop?tab=reviews" },
    { name: "phone-reviews", path: "/p/platform?tab=reviews", phone: true },
    // What the review read: a file of the pull request it reviewed.
    {
      name: "phone-diff",
      path: session("platform#438"),
      phone: true,
      act: (page) => openDiff(page, "src/billing/charge.rs", { phone: true }),
    },
  ],
  memory: [
    {
      name: "notes",
      path: session("platform#437"),
      act: (page) => scrollTo(page, "Remembered for later agents", 200),
    },
    { name: "retro", path: session("shop#224"), act: (page) => scrollTo(page, "Retrospective", 140) },
    { name: "memory", path: "/p/platform?tab=memory" },
    { name: "retrospectives", path: "/p/shop?tab=retrospectives" },
    { name: "phone-memory", path: "/p/platform?tab=memory", phone: true },
    { name: "phone-retrospectives", path: "/p/shop?tab=retrospectives", phone: true },
    // Platform's notes on a phone, as the features overview shows them in a phone of its own.
    {
      name: "card",
      path: "/p/platform?tab=memory",
      phone: true,
      act: (page) => scrollTo(page, "Answer Stripe", 104),
      card: [0, 0, 390, 788],
      cardOnly: true,
    },
  ],
  runners: [
    { name: "runners", path: "/w/romanesco?tab=runners", card: [418, 462, 720, 400] },
    { name: "milkfloat", path: "/w/milkfloat?tab=runners" },
    { name: "phone-runners", path: "/w/romanesco?tab=runners", phone: true },
    // Noor's MacBook and fika, the runners of hers that milkfloat's sessions run on.
    {
      name: "phone-milkfloat",
      path: "/w/milkfloat?tab=runners",
      phone: true,
      act: (page) => scrollTo(page, "Register a machine", 70),
    },
  ],
  schedules: [
    { name: "schedules", path: "/p/shop?tab=schedules" },
    { name: "empty", path: "/p/driver-app?tab=schedules" },
    {
      name: "editor",
      path: "/p/driver-app?tab=schedules",
      act: async (page) => {
        await page.getByRole("button", { name: /Update dependencies/ }).click();
        await page.waitForTimeout(600);
      },
      card: [438, 446, 620, 280],
    },
    { name: "scheduled", path: session("shop#227"), act: (page) => page.evaluate(() => window.scrollTo(0, 0)) },
    { name: "phone-schedules", path: "/p/shop?tab=schedules", phone: true },
    { name: "phone-empty", path: "/p/driver-app?tab=schedules", phone: true },
  ],
  "access-tokens": [
    { name: "tokens", path: "/settings?tab=tokens" },
    { name: "new", path: "/settings?tab=tokens", act: newToken, card: [394, 210, 500, 320] },
    {
      name: "confirm",
      path: "/settings?tab=tokens",
      act: async (page) => {
        const dialog = await newToken(page);
        await dialog.getByRole("button", { name: "Create token" }).click();
        await page.getByText("Confirm it's you").first().waitFor();
        await page.waitForTimeout(600);
      },
    },
    { name: "phone-tokens", path: "/settings?tab=tokens", phone: true },
  ],
  team: [
    { name: "people", path: "/w/romanesco?tab=people", card: [418, 350, 560, 470] },
    { name: "members", path: "/p/shop?tab=members" },
    { name: "project", path: "/p/shop" },
    { name: "watching", path: session("shop#231"), settle: 2500, act: (page) => railTo(page, "Watching (") },
    { name: "login", path: "/login", signedOut: true },
    { name: "phone-people", path: "/w/romanesco?tab=people", phone: true, act: showTab },
    { name: "phone-members", path: "/p/shop?tab=members", phone: true, act: showTab },
  ],
  "sign-in": [
    { name: "login", path: "/login", signedOut: true, card: [500, 150, 560, 440] },
    {
      name: "two-factor",
      path: "/login",
      signedOut: true,
      act: async (page) => {
        await page.fill("input[type=email]", manifest.email);
        await page.fill("input[type=password]", manifest.password);
        await page.click("button[type=submit]");
        await page.locator('input[autocomplete="one-time-code"]').waitFor();
        await page.waitForTimeout(500);
      },
    },
    { name: "security", path: "/settings?tab=security", act: (page) => scrollTo(page, "Passkeys", 120) },
    {
      name: "confirm",
      path: "/settings?tab=data",
      act: async (page) => {
        await page.getByRole("link", { name: /Export your data/ }).click();
        await page.getByText("Confirm it's you").first().waitFor();
        await page.getByRole("button", { name: "Use your authenticator app" }).click();
        await page.waitForTimeout(600);
      },
    },
    { name: "phone-login", path: "/login", phone: true, signedOut: true },
    {
      name: "phone-security",
      path: "/settings?tab=security",
      phone: true,
      act: (page) => scrollTo(page, "Passkeys", 80),
    },
  ],
};

// What to shoot: a name, a path, and how long the page takes to settle.
const shots = features
  ? Object.entries(pages).flatMap(([p, list]) => list.map((s) => ({ ...s, page: p, name: `${p}/${s.name}` })))
  : [
      { name: "floor", path: "/", settle: 3000 },
      { name: "shop", path: "/p/shop" },
      { name: "session", path: session("shop#231"), settle: 2500 },
      { name: "waiting", path: session("platform#445"), settle: 2000 },
      { name: "reviewed", path: session("shop#224"), settle: 2000 },
      { name: "reviews", path: "/p/platform?tab=reviews" },
      { name: "memory", path: "/p/platform?tab=memory" },
      { name: "retrospectives", path: "/p/shop?tab=retrospectives" },
      { name: "schedules", path: "/p/shop?tab=schedules" },
      { name: "tickets", path: "/p/shop?tab=tickets" },
      { name: "repositories", path: "/p/shop?tab=repositories" },
      { name: "runners", path: "/w/romanesco?tab=runners" },
      { name: "people", path: "/w/romanesco?tab=people" },
      { name: "milkfloat", path: "/w/milkfloat" },
      { name: "settings", path: "/settings" },
      { name: "phone-session", path: session("shop#231"), phone: true, settle: 2500 },
      { name: "phone-floor", path: "/", phone: true, settle: 3000 },
    // platform#445's agent asks whether a dead letter should page someone, with two answers to click.
    { name: "asking", path: session("platform#445"), settle: 2500 },
    { name: "phone-asking", path: session("platform#445"), phone: true, settle: 2500 },
    ];
for (let i = 0; i < args.length; i++) {
  if (args[i] !== "--page" || !args[i + 1]) continue;
  const at = args[i + 1].indexOf("=");
  const [name, raw] = [args[i + 1].slice(0, at), args[i + 1].slice(at + 1)];
  const p = raw.replace(/\{([^}]+)\}/g, (_, ref) => manifest.sessions[ref] ?? ref);
  shots.push({ name, path: p.startsWith("/") ? p : `/s/${p}`, phone: name.startsWith("phone"), settle: 2000 });
  if (!only.includes(name)) only.push(name);
}
// `--only` takes shots' names, and with `--features` whole pages too.
const wanted = shots.filter(
  (s) =>
    s.path &&
    !s.path.endsWith("/s/undefined") &&
    (only.length === 0 || only.includes(s.name) || (s.page && only.includes(s.page))),
);

/** A page as Noor sees it, in `theme`, at the moment of the shots: signed in, unless `signedOut`. */
async function noor(browser, { theme, phone, flowing, signedOut }) {
  const context = await browser.newContext({
    colorScheme: theme,
    reducedMotion: "reduce",
    timezoneId: manifest.zone ?? "Europe/Stockholm",
    locale: "en-GB",
    // fctl.sh's phones draw iOS's status bar above the screenshot (58 of 390):
    // the app gets the rest of an iPhone 15's screen, 852 × 393 as 846 × 390.
    viewport: phone ? { width: 390, height: features ? 788 : 844 } : open ? null : { width, height },
    deviceScaleFactor: open ? undefined : 2,
    isMobile: phone,
    hasTouch: phone,
  });
  // Held at the moment of the shots, or, to look around, going on from it.
  if (manifest.anchor) {
    if (flowing) await context.clock.install({ time: manifest.anchor });
    else await context.clock.setFixedTime(manifest.anchor);
  }
  await context.addInitScript(() => {
    let seed = 42;
    Math.random = () => (seed = (seed * 16807) % 2147483647) / 2147483647;
    // Sessions in the new view, which reads in turns (web/src/lib/session-view.ts).
    try {
      localStorage.setItem("fctl:session-view", "new");
    } catch {}
  });
  // The cast's avatars in place of GitHub's; bots' logos stay GitHub's.
  await context.route("https://avatars.githubusercontent.com/**", (route) => {
    const login = decodeURIComponent(new URL(route.request().url()).pathname.slice(1));
    const file = path.join(avatars, `${login}.png`);
    return existsSync(file) ? route.fulfill({ path: file, contentType: "image/png" }) : route.continue();
  });
  // An instance with a GitHub App and its webhook, and Google sign-in, which
  // the stack can't have, as app.fctl.sh has them: no notes in settings that
  // say it lacks them, and Google's button where people sign in.
  await context.route("**/api/v1/instance", async (route) => {
    const response = await route.fetch();
    const body = await response.json();
    Object.assign(body.features ?? {}, {
      google: true,
      github: true,
      github_webhook: true,
      publish_screenshots: true,
      linear_webhook: true,
    });
    await route.fulfill({ response, json: body });
  });
  await context.route("**/api/v1/github/repos?*", (route) => route.fulfill({ json: { repos: [] } }));
  // Review agents need a GitHub App, which the stack has none of.
  await context.route("**/api/v1/projects/*/review-agents", async (route) => {
    const response = await route.fetch();
    const body = await response.json();
    delete body.problem;
    await route.fulfill({ response, json: body });
  });
  // Runners' hosts say they're alive every 15 seconds, at the real time:
  // the online ones were seen just before the moment of the shots.
  if (manifest.anchor) {
    await context.route(/\/api\/v1\/(workspaces\/[^/]+\/)?runners$/, async (route) => {
      if (route.request().method() !== "GET") return route.continue();
      const response = await route.fetch();
      const body = await response.json();
      for (const [i, r] of (body.runners ?? []).entries()) {
        if (r.last_seen_at > manifest.anchor) r.last_seen_at = manifest.anchor - 4000 - i * 3000;
      }
      await route.fulfill({ response, json: body });
    });
  }
  // Linear's teams, which the stack can't ask Linear for.
  await context.route("**/api/v1/workspaces/*/linear/*/teams", (route) => {
    const [, , , , id] = new URL(route.request().url()).pathname.split("/");
    return route.fulfill({ json: { teams: manifest.teams[id] ?? [] } });
  });
  const page = await context.newPage();
  await ready(context, page);
  if (signedOut) return { context, page };
  if (manifest.cookie) {
    // Her sign-in from the seed: she has a second factor, and a sign-in
    // with it would count as confirming it's her for 15 minutes.
    const at = manifest.cookie.indexOf("=");
    const [name, value] = [manifest.cookie.slice(0, at), manifest.cookie.slice(at + 1)];
    const { hostname } = new URL(manifest.app);
    await context.addCookies([{ name, value, domain: hostname, path: "/", secure: true, httpOnly: true, sameSite: "Lax" }]);
    await page.goto(new URL("/", manifest.app).href);
  } else {
    await page.goto(new URL("/login", manifest.app).href);
    await page.fill("input[type=email]", manifest.email);
    await page.fill("input[type=password]", manifest.password);
    await page.click("button[type=submit]");
    await page.waitForURL((u) => u.pathname !== "/login");
  }
  return { context, page };
}

/**
 * A PNG as WebP, `width` pixels wide at most, with Chromium's encoder: all
 * of it, or the part `crop` says ([x, y, width, height], fractions of it).
 */
async function webp(converter, png, maxWidth, crop = [0, 0, 1, 1]) {
  const data = await converter.evaluate(
    async ({ b64, maxWidth, crop }) => {
      const img = new Image();
      img.src = `data:image/png;base64,${b64}`;
      await img.decode();
      const [sx, sy, sw, sh] = [
        crop[0] * img.naturalWidth,
        crop[1] * img.naturalHeight,
        crop[2] * img.naturalWidth,
        crop[3] * img.naturalHeight,
      ];
      const w = Math.round(Math.min(maxWidth, sw));
      const h = Math.round((sh * w) / sw);
      const c = document.createElement("canvas");
      c.width = w;
      c.height = h;
      const ctx = c.getContext("2d");
      ctx.imageSmoothingQuality = "high";
      ctx.drawImage(img, sx, sy, sw, sh, 0, 0, w, h);
      return { url: c.toDataURL("image/webp", 0.86), w, h };
    },
    { b64: png.toString("base64"), maxWidth, crop },
  );
  return { bytes: Buffer.from(data.url.split(",")[1], "base64"), width: data.w, height: data.h };
}

if (open) {
  const browser = await chromium.launch({ headless: false });
  const { page } = await noor(browser, { theme: themes[0], phone: false, flowing: true });
  console.log(`Signed in as ${manifest.email} at 09:10 in ${manifest.zone}. Close the browser to stop.`);
  await page.waitForEvent("close", { timeout: 0 });
  await browser.close();
} else {
  mkdirSync(out, { recursive: true });
  const browser = await chromium.launch();
  const converter = features ? await browser.newPage() : null;
  for (const theme of themes) {
    for (const phone of [false, true]) {
      for (const signedOut of [false, true]) {
        const mine = wanted.filter((s) => Boolean(s.phone) === phone && Boolean(s.signedOut) === signedOut);
        if (mine.length === 0) continue;
        const { context, page } = await noor(browser, { theme, phone, flowing: false, signedOut });
        for (const shot of mine) {
          // A page of its own for each shot that changes what it shows, or the window's size.
          const tab = shot.act || shot.size ? await context.newPage() : page;
          if (tab !== page) await ready(context, tab);
          if (shot.size) {
            const [w, h] = shot.size.split("x").map(Number);
            await tab.setViewportSize({ width: w, height: h });
          }
          // A page whose requests never go quiet still gets its shot, after a while.
          const url = new URL(shot.path, manifest.app).href;
          await tab.goto(url, { waitUntil: "networkidle", timeout: 20_000 }).catch(async () => {
            console.error(`${shot.name} (${theme}): still loading after 20 s, shooting it anyway`);
            await tab.goto(url, { waitUntil: "load" });
          });
          await tab.waitForTimeout(shot.settle ?? 1500);
          try {
            await showTab(tab);
            if (shot.act) await shot.act(tab);
          } catch (e) {
            console.error(`${shot.name} (${theme}): ${e.message.split("\n")[0]}`);
          }
          await dress(tab);
          // Where its card is, measured on the page the shot shows: one it can't find is left
          // out, as a failed act is, and the run goes on.
          const card =
            typeof shot.card === "function"
              ? await shot.card(tab).catch((e) => {
                  console.error(`${shot.name} (${theme}): no card: ${e.message.split("\n")[0]}`);
                  return undefined;
                })
              : shot.card;
          const png = await tab.screenshot();
          if (features) {
            mkdirSync(path.join(out, shot.page), { recursive: true });
            if (!shot.cardOnly) {
              const file = path.join(out, `${shot.name}-${theme}.webp`);
              const w = await webp(converter, png, phone ? 780 : 2200);
              writeFileSync(file, w.bytes);
              console.log(`${file} ${w.width}×${w.height}`);
            }
            // Its page's tile on the features page: the part of it that `card`
            // says, [x, y, width, height] in CSS pixels, at twice that size.
            if (card) {
              const view = tab.viewportSize();
              const [x, y, w, h] = card;
              const crop = [x / view.width, y / view.height, w / view.width, h / view.height];
              const c = await webp(converter, png, w * 2, crop);
              const file = path.join(out, shot.page, `card-${theme}.webp`);
              writeFileSync(file, c.bytes);
              console.log(`${file} ${c.width}×${c.height}`);
            }
          } else {
            const file = path.join(out, `${shot.name}-${theme}.png`);
            writeFileSync(file, png);
            console.log(file);
          }
          if (tab !== page) await tab.close();
        }
        await context.close();
      }
    }
  }
  await browser.close();
}
