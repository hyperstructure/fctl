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
// is taken in both themes; the phone ones at 390 × 844.

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

/** Opens the transcript's setup row. */
async function openSetup(page) {
  await page.locator('button:has-text("Sandbox ready in")').first().click();
  await page.waitForTimeout(500);
}

/** Settings' new access token, filled in as Noor makes one for the team's chat bot. */
async function newToken(page) {
  await scrollTo(page, "Access tokens", 120);
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
// show and how to get there. `act` runs once the page has settled.
const pages = {
  sessions: [
    { name: "floor", path: "/", settle: 3000, thumb: true },
    { name: "session", path: session("shop#231"), settle: 2500 },
    {
      name: "follow-up",
      path: session("driver-app#75"),
      act: (page) => scrollTo(page, "A check failed", 140),
    },
    { name: "phone-floor", path: "/", phone: true, settle: 3000 },
    { name: "phone-session", path: session("shop#231"), phone: true, settle: 2500 },
  ],
  workspaces: [
    { name: "workspace", path: "/w/romanesco", thumb: true },
    { name: "project", path: "/p/shop" },
    { name: "settings", path: "/p/shop?tab=settings" },
    { name: "people", path: "/w/romanesco?tab=people" },
    { name: "members", path: "/p/shop?tab=members" },
    { name: "workspace-settings", path: "/w/romanesco?tab=settings" },
    { name: "phone-workspace", path: "/w/romanesco", phone: true },
    { name: "phone-project", path: "/p/shop", phone: true },
  ],
  sandboxes: [
    { name: "setup", path: session("shop#231"), act: openSetup, thumb: true },
    { name: "build", path: session("milkfloat#89"), act: openSetup },
    {
      name: "settings",
      path: "/p/milkfloat?tab=settings",
      act: (page) => scrollTo(page, "Setup script", 180),
    },
    { name: "runners", path: "/w/romanesco?tab=runners", thumb: true },
    { name: "phone-setup", path: session("shop#231"), phone: true, act: openSetup },
  ],
  linear: [
    { name: "delegated", path: session("shop#231"), act: (page) => page.evaluate(() => window.scrollTo(0, 0)), thumb: true },
    {
      name: "exploration",
      path: session("shop#212"),
      act: (page) => scrollTo(page, "Replied on SHOP-398", 220),
    },
    { name: "tickets", path: "/p/shop?tab=tickets" },
    { name: "workspace", path: "/w/romanesco?tab=linear" },
    {
      name: "phone-delegated",
      path: session("driver-app#75"),
      phone: true,
      act: (page) => page.evaluate(() => window.scrollTo(0, 0)),
    },
  ],
  "review-agents": [
    { name: "reviews", path: "/p/platform?tab=reviews", thumb: true },
    { name: "off", path: "/p/driver-app?tab=reviews" },
    { name: "review", path: session("platform#438"), settle: 2000 },
    {
      name: "findings",
      path: session("platform#437"),
      act: (page) => scrollTo(page, "Two workers can still both charge the card", 260),
    },
    { name: "outside", path: session("milkfloat#94"), settle: 2000 },
    { name: "phone-reviews", path: "/p/platform?tab=reviews", phone: true },
    { name: "phone-review", path: session("platform#438"), phone: true },
  ],
  memory: [
    {
      name: "notes",
      path: session("platform#437"),
      act: (page) => scrollTo(page, "Remembered for later agents", 200),
    },
    { name: "retro", path: session("shop#224"), act: (page) => scrollTo(page, "Retrospective", 140) },
    { name: "memory", path: "/p/platform?tab=memory", thumb: true },
    { name: "retrospectives", path: "/p/shop?tab=retrospectives" },
    { name: "phone-memory", path: "/p/platform?tab=memory", phone: true },
    { name: "phone-retrospectives", path: "/p/shop?tab=retrospectives", phone: true },
  ],
  runners: [
    { name: "runners", path: "/w/romanesco?tab=runners" },
    { name: "milkfloat", path: "/w/milkfloat?tab=runners" },
    { name: "phone-runners", path: "/w/romanesco?tab=runners", phone: true },
  ],
  schedules: [
    { name: "schedules", path: "/p/shop?tab=schedules", thumb: true },
    { name: "empty", path: "/p/driver-app?tab=schedules" },
    {
      name: "editor",
      path: "/p/driver-app?tab=schedules",
      act: async (page) => {
        await page.getByRole("button", { name: /Update dependencies/ }).click();
        await page.waitForTimeout(600);
      },
    },
    { name: "scheduled", path: session("shop#227"), act: (page) => page.evaluate(() => window.scrollTo(0, 0)) },
    { name: "phone-schedules", path: "/p/shop?tab=schedules", phone: true },
    { name: "phone-empty", path: "/p/driver-app?tab=schedules", phone: true },
  ],
  "access-tokens": [
    { name: "tokens", path: "/settings", act: (page) => scrollTo(page, "Devices", 120), thumb: true },
    { name: "new", path: "/settings", act: newToken },
    {
      name: "confirm",
      path: "/settings",
      act: async (page) => {
        const dialog = await newToken(page);
        await dialog.getByRole("button", { name: "Create token" }).click();
        await page.getByText("Confirm it's you").first().waitFor();
        await page.waitForTimeout(600);
      },
    },
    { name: "phone-tokens", path: "/settings", phone: true, act: (page) => scrollTo(page, "Devices", 80) },
  ],
  "sign-in": [
    { name: "login", path: "/login", signedOut: true, thumb: true },
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
    { name: "security", path: "/settings", act: (page) => scrollTo(page, "Passkeys", 120) },
    {
      name: "confirm",
      path: "/settings",
      act: async (page) => {
        await scrollTo(page, "Your data", 160);
        await page.getByRole("link", { name: /Export your data/ }).click();
        await page.getByText("Confirm it's you").first().waitFor();
        await page.getByRole("button", { name: "Use your authenticator app" }).click();
        await page.waitForTimeout(600);
      },
    },
    { name: "phone-login", path: "/login", phone: true, signedOut: true },
    { name: "phone-security", path: "/settings", phone: true, act: (page) => scrollTo(page, "Passkeys", 80) },
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
    viewport: phone ? { width: 390, height: 844 } : open ? null : { width, height },
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
  });
  // The cast's avatars in place of GitHub's; bots' logos stay GitHub's.
  await context.route("https://avatars.githubusercontent.com/**", (route) => {
    const login = decodeURIComponent(new URL(route.request().url()).pathname.slice(1));
    const file = path.join(avatars, `${login}.png`);
    return existsSync(file) ? route.fulfill({ path: file, contentType: "image/png" }) : route.continue();
  });
  // An instance with a GitHub App and its webhook, which the stack can't
  // have, as app.fctl.sh has them: no notes in settings that say it lacks them.
  await context.route("**/api/v1/instance", async (route) => {
    const response = await route.fetch();
    const body = await response.json();
    Object.assign(body.features ?? {}, {
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
  const sizes = {};
  for (const theme of themes) {
    for (const phone of [false, true]) {
      for (const signedOut of [false, true]) {
        const mine = wanted.filter((s) => Boolean(s.phone) === phone && Boolean(s.signedOut) === signedOut);
        if (mine.length === 0) continue;
        const { context, page } = await noor(browser, { theme, phone, flowing: false, signedOut });
        for (const shot of mine) {
          // A page of its own for each shot that changes what it shows.
          const tab = shot.act ? await context.newPage() : page;
          if (tab !== page) await ready(context, tab);
          // A page whose requests never go quiet still gets its shot, after a while.
          const url = new URL(shot.path, manifest.app).href;
          await tab.goto(url, { waitUntil: "networkidle", timeout: 20_000 }).catch(async () => {
            console.error(`${shot.name} (${theme}): still loading after 20 s, shooting it anyway`);
            await tab.goto(url, { waitUntil: "load" });
          });
          await tab.waitForTimeout(shot.settle ?? 1500);
          try {
            if (shot.act) await shot.act(tab);
          } catch (e) {
            console.error(`${shot.name} (${theme}): ${e.message.split("\n")[0]}`);
          }
          await dress(tab);
          const png = await tab.screenshot();
          if (features) {
            const file = path.join(out, `${shot.name}-${theme}.webp`);
            mkdirSync(path.dirname(file), { recursive: true });
            const w = await webp(converter, png, phone ? 780 : 2200);
            writeFileSync(file, w.bytes);
            sizes[`${shot.name}-${theme}`] = [w.width, w.height];
            console.log(`${file} ${w.width}×${w.height}`);
            // The features page's card for it: the app's content, without
            // the sidebar where it has one, small.
            if (shot.thumb) {
              const crop = shot.signedOut ? [0.1, 0.06, 0.8, 0.8] : [0.17, 0, 0.8, 0.8];
              const t = await webp(converter, png, 720, crop);
              writeFileSync(path.join(out, shot.page, `thumb-${theme}.webp`), t.bytes);
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
