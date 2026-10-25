// Screenshots of the demo cast in `cargo xtask demo`'s stack, as Noor sees it.
//
//   cd ~/screenshots   # where Playwright is installed (the screenshots skill)
//   node <repo>/xtask/demo/shoot.mjs [--out DIR] [--only floor,session] [--theme dark|light] [--size 1440x900]
//   node <repo>/xtask/demo/shoot.mjs --page NAME=PATH   # more pages: {project#number} in PATH is that session's
//   node <repo>/xtask/demo/shoot.mjs --open             # a browser to look around or record in, on your machine
//
// It signs in as Noor, shows the cast's avatars where the app loads GitHub's
// (avatars.githubusercontent.com/<login>), answers what the stack can't ask
// GitHub or Linear (review agents' problem, Linear's teams), and holds the
// browser's clock at the moment of the shots, 09:10 in the zone `cargo xtask
// demo` picked (.fctl-demo/shots.json), so that the app shows the story's
// times. Each shot is taken in both themes; the phone ones at 390 × 844.

import { existsSync, mkdirSync, readFileSync } from "node:fs";
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
const out = path.resolve(opt("--out", path.join(process.cwd(), "demo")));
const only = opt("--only", "")
  .split(",")
  .filter(Boolean);
const themes = opt("--theme", "dark,light").split(",");
const [width, height] = opt("--size", "1440x900").split("x").map(Number);
const open = args.includes("--open");
const manifest = JSON.parse(readFileSync(path.join(root, ".fctl-demo/shots.json"), "utf8"));
const avatars = path.join(here, "avatars");

const session = (ref) => `/s/${manifest.sessions[ref]}`;
// What to shoot: a name, a path, and how long the page takes to settle.
const shots = [
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
const wanted = shots.filter(
  (s) => s.path && !s.path.endsWith("/s/undefined") && (only.length === 0 || only.includes(s.name)),
);

/** A signed-in page as Noor sees it, in `theme`, at the moment of the shots. */
async function noor(browser, { theme, phone, flowing }) {
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
  await page.goto(new URL("/login", manifest.app).href);
  await page.fill("input[type=email]", manifest.email);
  await page.fill("input[type=password]", manifest.password);
  await page.click("button[type=submit]");
  await page.waitForURL((u) => u.pathname !== "/login");
  return { context, page };
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
  for (const theme of themes) {
    for (const phone of [false, true]) {
      const mine = wanted.filter((s) => Boolean(s.phone) === phone);
      if (mine.length === 0) continue;
      const { context, page } = await noor(browser, { theme, phone, flowing: false });
      for (const shot of mine) {
        await page.goto(new URL(shot.path, manifest.app).href, { waitUntil: "networkidle" });
        await page.waitForTimeout(shot.settle ?? 1500);
        const file = path.join(out, `${shot.name}-${theme}.png`);
        await page.screenshot({ path: file });
        console.log(file);
      }
      await context.close();
    }
  }
  await browser.close();
}
