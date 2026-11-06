// The video of a session's diffs following its agent, for fctl.sh's Sessions
// page: platform#437's agent fixes a double charge, and its diffs change as
// the session page shows them while it works (web/src/components/diff-play.ts):
// a new file comes in, single lines are written over where the reader looks,
// files arrive above and below while the lines in view stay put, and the
// reader's pointer takes the button to the changes below.
//
//   cd ~/screenshots   # where Playwright is installed (the screenshots skill)
//   node <repo>/xtask/demo/diff-video.mjs --out ~/repos/hyperstructure/fctl-site/src/features/shots/sessions
//   node <repo>/xtask/demo/diff-video.mjs --theme dark --quick   # frames 4 a second, small: to check the story
//
// It needs ffmpeg (with libx264, libvpx-vp9 and libwebp, but not for --quick)
// and the web app's packages (npm ci). No stack runs: the session's diffs are what git makes of its
// transcript's edits (`cargo xtask demo steps platform-437`) on its
// repository's stand-in (xtask/demo/repos), version after version, and the
// app's own dialog shows them on a page of their own (web/demo/diffs.html),
// served by Vite. Each frame is taken at a set moment of a clock that stands
// still between them, so nothing depends on how fast the machine is. It
// writes, for each theme, `diff-live-<theme>.mp4` and `.webm` (2200 × 1376,
// 60 fps, looping without a jump) and the poster `diff-<theme>.webp`, which
// fctl.sh's page shows before the video plays and where motion is reduced.
//
// The story: the transcript's edits, with some of them as the smaller steps
// an agent takes (STORY), at the seconds in LANDS. It checks that they end
// where the transcript does, line for line.

import { execFileSync, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  cpSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  renameSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
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
const out = path.resolve(opt("--out", path.join(process.cwd(), "diff-video")));
const themes = opt("--theme", "dark,light").split(",");
const quick = args.includes("--quick");
const kept = opt("--frames", "");

const TRANSCRIPT = "platform-437";
const BRANCH = "fctl/437-find-why-two-customers-were-charged-twice";
const OPENS_AT = "migrations/0140_charge_keys.sql";
const MIGRATION = "migrations/0140_charge_keys.sql";
const CHARGE = "src/billing/charge.rs";

/**
 * The agent's edits as the video shows them: the transcript's, some of them
 * as the smaller steps an agent takes. The migration first takes charge_id
 * as required, then lets a key be claimed before there is a charge; a doc
 * comment gets a word put right; and the second turn's change to
 * charge_once comes in five steps. Each change names what it does to the
 * transcript's steps.
 */
function story(steps) {
  const at = (pred, what) => {
    const i = steps.findIndex(pred);
    if (i < 0) throw new Error(`the transcript has no ${what}: the story needs telling again`);
    return i;
  };
  const replace = (s, from, to) => {
    if (!s.includes(from)) throw new Error(`the transcript's step has no ${JSON.stringify(from)}`);
    return s.replace(from, to);
  };
  const m = at((s) => s.write === MIGRATION, "migration");
  steps[m] = {
    ...steps[m],
    content: replace(steps[m].content, "charge_id    TEXT,", "charge_id    TEXT NOT NULL,"),
  };
  const c1 = at((s) => s.edit === CHARGE, "first change to charge.rs");
  steps.splice(c1 + 1, 0, { edit: MIGRATION, old: "charge_id    TEXT NOT NULL,", new: "charge_id    TEXT," });
  steps[c1] = {
    ...steps[c1],
    new: replace(steps[c1].new, "for a period came to.", "for a period comes to."),
  };
  const q = at((s) => s.write?.startsWith(".sqlx/query-a41e"), "first sqlx query");
  steps.splice(q + 1, 0, { edit: CHARGE, old: "for a period comes to.", new: "for a period came to." });
  const c2 = steps.findLastIndex((s) => s.edit === CHARGE);
  const whole = steps[c2];
  const parts = [
    [
      "    if db.charge_for(subscription, period).await?.is_some() {",
      "    if !db.claim_charge_key(subscription, period).await? {",
    ],
    [
      "        return Ok(Charged::Already);\n    }\n",
      '        return Ok(Charged::Already);\n    }\n    let key = format!("charge:{subscription}:{period}");\n',
    ],
    [
      "    let charge = stripe::charge(subscription, amount).await?;",
      "    let charge = stripe::charge_with_key(subscription, amount, &key).await?;",
    ],
    [
      "    let charge = stripe::charge_with_key(subscription, amount, &key).await?;",
      [
        "    let charge = match stripe::charge_with_key(subscription, amount, &key).await {",
        "        Ok(charge) => charge,",
        "        Err(e) => {",
        "            // Declined, or Stripe unreachable: the next try may charge.",
        "            db.release_charge_key(subscription, period).await?;",
        "            return Err(e);",
        "        }",
        "    };",
      ].join("\n"),
    ],
    [
      "    if !db.claim_charge_key(subscription, period).await? {",
      [
        "    // Whoever inserts the key charges; anyone after finds it taken. Stripe",
        "    // gets the key too, so a call retried after a timeout can't charge again.",
        "    if !db.claim_charge_key(subscription, period).await? {",
      ].join("\n"),
    ],
  ];
  steps.splice(c2, 1, ...parts.map(([old, now]) => ({ edit: CHARGE, old, new: now })));
  return { steps, whole };
}

/** The agent's refreshes as the video shows them: [second, version]. Versions count from 0, the first step's. */
const LANDS = [
  [1.2, 1], // charge.rs: the first fix comes in under the migration
  [3.0, 2], // the migration: charge_id may wait for the charge (one line)
  [4.3, 3], // src/db.rs, below
  [5.6, 4], // tests/charge_once.rs, new, below
  [6.8, 5], // a .sqlx query, new, above everything
  [8.1, 6], // charge.rs: a word in a doc comment (one line)
  [9.2, 7], // tests/charge_once.rs again
  [10.1, 8], // tests/charge_race.rs, new
  [11.2, 9], // src/db.rs again
  [13.6, 10], // charge.rs: the key is claimed (one line)
  [14.6, 11], // charge.rs: the key itself (a line)
  [15.5, 12], // charge.rs: Stripe gets the key (one line)
  [16.7, 13], // charge.rs: a failed charge gives the key back (a block)
  [18.0, 14], // charge.rs: why (two lines)
  [19.1, 15], // .sqlx, above
  [19.8, 16], // .sqlx, above
];
/** The reader scrolls down to charge.rs as they read, just before the agent rewrites it. */
const READ = 12.2;
const READ_GLIDE = 0.95;
/** Their pointer comes in, takes the button to the changes below, and goes. */
const CURSOR_IN = 20.7;
const CLICK = 21.6;
const GLIDE = 0.85;
const CURSOR_OUT = 23.4;
const END = 25.0;
/** The poster's moment: charge.rs as the agent left it, every file in the list, before the pointer comes. */
const POSTER = 20.5;
const FPS = 60;

/** The session's diffs after each step of `steps`, as fctl-agent reads them (crates/fctl-agent/src/workspace.rs). */
function versions(repo, steps) {
  const dir = mkdtempSync(path.join(tmpdir(), "fctl-diff-video-"));
  cpSync(path.join(here, "repos", repo), dir, { recursive: true });
  // As the seed gives the stand-in's manifests their names back.
  for (const f of readdirSync(dir, { recursive: true })) {
    if (f.endsWith(".stand-in"))
      renameSync(path.join(dir, f), path.join(dir, f.slice(0, -".stand-in".length)));
  }
  const git = (...a) => execFileSync("git", a, { cwd: dir, encoding: "utf8" });
  git("init", "-q", "-b", "main");
  git("-c", "user.email=demo@example.com", "-c", "user.name=Demo", "add", "-A");
  git("-c", "user.email=demo@example.com", "-c", "user.name=Demo", "commit", "-q", "-m", "base");
  const base = git("rev-parse", "HEAD").trim();
  const patchOf = (raw) => raw.slice(raw.indexOf("\n@@") + 1).replace(/\n$/, "");
  const snap = () => {
    const files = [];
    const status = new Map(
      git("diff", "--name-status", base)
        .split("\n")
        .filter(Boolean)
        .map((l) => l.split("\t").reverse()),
    );
    for (const l of git("diff", "--numstat", base).split("\n").filter(Boolean)) {
      const [a, d, p] = l.split("\t");
      const patch = patchOf(git("diff", "--no-color", "-U3", base, "--", p));
      files.push({
        path: p,
        status: status.get(p) === "A" ? "added" : "modified",
        additions: +a,
        deletions: +d,
        patch,
      });
    }
    for (const p of git("ls-files", "--others", "--exclude-standard").split("\n").filter(Boolean)) {
      const r = spawnSync("git", ["diff", "--no-color", "--no-index", "/dev/null", p], {
        cwd: dir,
        encoding: "utf8",
      });
      const patch = patchOf(r.stdout);
      const additions = patch.split("\n").filter((x) => x.startsWith("+")).length;
      files.push({ path: p, status: "untracked", additions, deletions: 0, patch });
    }
    files.sort((a, b) => (a.path < b.path ? -1 : 1));
    for (const f of files) f.digest = createHash("sha1").update(f.patch).digest("hex").slice(0, 12);
    return files;
  };
  const all = [];
  try {
    for (const s of steps) {
      const file = path.join(dir, s.edit ?? s.write);
      if (s.write !== undefined) {
        mkdirSync(path.dirname(file), { recursive: true });
        writeFileSync(file, s.content);
      } else {
        const text = readFileSync(file, "utf8");
        if (!text.includes(s.old)) throw new Error(`${s.edit} has no ${JSON.stringify(s.old.slice(0, 60))}`);
        writeFileSync(file, s.replace_all ? text.replaceAll(s.old, s.new) : text.replace(s.old, s.new));
      }
      all.push(snap());
    }
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
  return all;
}

/** Records one theme's frames into `dir`, PNGs at twice the CSS pixels (or a few small ones, `quick`). */
async function record(browser, url, demo, theme, dir) {
  const context = await browser.newContext({
    viewport: { width: 1440, height: 900 },
    deviceScaleFactor: quick ? 1 : 2,
    colorScheme: theme,
    reducedMotion: "no-preference",
    // Noor's Mac: its shortcuts say ⌘.
    userAgent:
      "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/142.0.0.0 Safari/537.36",
  });
  await context.addInitScript(
    (d) => {
      window.fctlDemoDiffs = d;
    },
    { ...demo, theme },
  );
  const page = await context.newPage();
  page.on("pageerror", (e) => console.error(`${theme}: ${e.message}`));
  await page.clock.install();
  await page.goto(url);
  await page.waitForSelector("section[data-file] .diff-code span[style]");
  for (let i = 0; i < 40; i++) {
    await page.clock.runFor(100);
    await page.waitForTimeout(10);
  }
  await page.mouse.move(1430, 890);
  await page.clock.pauseAt((await page.evaluate(() => Date.now())) + 80);
  // What played as the dialog opened is done: it stays as it ended. The pointer goes in the
  // dialog's top layer, where a fixed element is placed from the dialog's corner (the transform
  // its opening left on it), and the views glide here rather than with the browser's smooth
  // scroll, which runs on the browser's clock, not this one.
  await page.evaluate(() => {
    window.fctlSkip = new Set(document.getAnimations());
    const dialog = document.querySelector("dialog");
    const c = document.createElement("div");
    c.id = "fctl-demo-cursor";
    c.style.cssText =
      "position:fixed;left:0;top:0;z-index:2147483647;pointer-events:none;opacity:0;transform-origin:3px 2px;filter:drop-shadow(0 2px 3px rgb(0 0 0 / 0.35))";
    c.innerHTML =
      '<svg width="22" height="30" viewBox="0 0 22 30"><path d="M2 1.5v23.2l6.1-5.8 3.9 9.1 4-1.7-3.9-8.9h8.4z" fill="#fff" stroke="#111" stroke-width="1.6" stroke-linejoin="round"/></svg>';
    dialog?.append(c);
    const body = document.querySelector("dialog [tabindex='0'].overflow-auto");
    const scrollTo = Element.prototype.scrollTo;
    Element.prototype.scrollTo = function (...a) {
      const o = a[0];
      if (this === body && o && typeof o === "object" && o.behavior && o.behavior !== "instant") {
        window.fctlGlide = { from: this.scrollTop, to: o.top };
        return;
      }
      return scrollTo.apply(this, a);
    };
  });
  const ease = (x) => (x < 0.5 ? 4 * x * x * x : 1 - (-2 * x + 2) ** 3 / 2);
  const settle = () =>
    page.evaluate(async () => {
      for (let i = 0; i < 8; i++)
        await new Promise((r) => {
          const ch = new MessageChannel();
          ch.port1.onmessage = () => r();
          ch.port2.postMessage(0);
        });
    });
  const header = () => page.evaluate(() => document.querySelector("dialog header")?.textContent);
  let next = 0;
  let glideAt;
  let glideFor = GLIDE;
  let pill;
  const frames = Math.round(END * FPS);
  for (let f = 0; f < frames; f++) {
    const s = f / FPS;
    while (next < LANDS.length && LANDS[next][0] <= s) {
      const before = await header();
      await page.evaluate((v) => window.fctlDemoStep?.(v), LANDS[next][1]);
      for (let i = 0; i < 40 && (await header()) === before; i++) await settle();
      next++;
    }
    if (Math.abs(s - READ) < 0.5 / FPS) {
      await page.evaluate((file) => {
        const b = document.querySelector("dialog [tabindex='0'].overflow-auto");
        const section = [...document.querySelectorAll("section[data-file]")].find((x) =>
          x.querySelector("h3")?.getAttribute("title")?.endsWith(file),
        );
        if (!b || !section) return;
        // As a trackpad would: the file the reader went to is theirs to leave.
        b.dispatchEvent(new WheelEvent("wheel", { deltaY: 120, bubbles: true }));
        window.fctlGlide = { from: b.scrollTop, to: section.offsetTop };
      }, CHARGE);
      glideAt = s;
      glideFor = READ_GLIDE;
    }
    // The pointer: in from the right, to the button, a click, away.
    if (s >= CURSOR_IN && s < CURSOR_OUT + 0.6) {
      pill ??= (await page.evaluate(() => {
        const b = document.querySelector(".diff-unseen[data-dir='below']")?.getBoundingClientRect();
        return b && { x: b.left + b.width * 0.42, y: b.top + b.height * 0.55 };
      })) ?? { x: 860, y: 860 };
      const start = { x: 1180, y: 610 };
      const k = ease(Math.min(1, Math.max(0, (s - CURSOR_IN) / (CLICK - 0.15 - CURSOR_IN))));
      let x = start.x + (pill.x - start.x) * k;
      let y = start.y + (pill.y - start.y) * k;
      if (s > CURSOR_OUT) {
        const k2 = ease(Math.min(1, (s - CURSOR_OUT) / 0.6));
        x += 70 * k2;
        y -= 40 * k2;
      }
      const pressed = s >= CLICK - 0.06 && s < CLICK + 0.1;
      const opacity =
        s < CURSOR_IN + 0.25
          ? (s - CURSOR_IN) / 0.25
          : s > CURSOR_OUT
            ? Math.max(0, 1 - (s - CURSOR_OUT) / 0.5)
            : 1;
      await page.evaluate(
        ({ x, y, pressed, opacity }) => {
          const c = document.getElementById("fctl-demo-cursor");
          const d = c?.parentElement;
          if (!c || !d) return;
          // From the dialog's padding box, where the pointer is placed.
          const r = d.getBoundingClientRect();
          const left = x - r.left - d.clientLeft;
          const top = y - r.top - d.clientTop;
          c.style.transform = `translate(${left - 3}px, ${top - 2}px) scale(${pressed ? 0.88 : 1})`;
          c.style.opacity = String(opacity);
        },
        { x, y, pressed, opacity },
      );
      await page.mouse.move(x, y);
      if (Math.abs(s - CLICK) < 0.5 / FPS) {
        await page.mouse.down();
        await page.mouse.up();
        glideAt = s;
        glideFor = GLIDE;
      }
    }
    const glide = await page.evaluate(() => window.fctlGlide);
    if (glide && glideAt !== undefined) {
      const k = ease(Math.min(1, (s - glideAt) / glideFor));
      await page.evaluate(
        (top) => {
          const b = document.querySelector("dialog [tabindex='0'].overflow-auto");
          if (b) b.scrollTop = top;
        },
        glide.from + (glide.to - glide.from) * k,
      );
      if (k >= 1) await page.evaluate(() => delete window.fctlGlide);
    }
    // The page's clock to this frame: its timers fire, its frames run, React commits.
    await page.clock.runFor(1000 / FPS);
    await settle();
    // Every animation to this frame's moment, from when it started.
    await page.evaluate((now) => {
      window.fctlStarts ??= new Map();
      for (const a of document.getAnimations()) {
        if (window.fctlSkip.has(a)) continue;
        let start = window.fctlStarts.get(a);
        if (start === undefined) {
          start = now;
          window.fctlStarts.set(a, start);
        }
        const at = now - start;
        const end = a.effect?.getComputedTiming().endTime;
        if (typeof end === "number" && Number.isFinite(end) && at >= end) {
          if (a.playState !== "finished") a.finish();
          continue;
        }
        a.pause();
        a.currentTime = at;
      }
    }, s * 1000);
    // A frame --quick doesn't keep is still drawn, as small as can be: some of what the dialog
    // does waits for the browser to draw (scroll events), and it would come at other frames.
    if (!quick || f % 15 === 0)
      await page.screenshot({ path: path.join(dir, `f${String(f).padStart(5, "0")}.png`) });
    else await page.screenshot({ clip: { x: 0, y: 0, width: 1, height: 1 } });
    if (f % (FPS * 5) === 0) console.log(`${theme}: ${s.toFixed(0)} s of ${END}`);
  }
  await context.close();
}

/** What encode() needs of ffmpeg, checked before minutes of recording rather than after. */
function encoders() {
  const need = ["libx264", "libvpx-vp9", "libwebp"];
  const r = spawnSync("ffmpeg", ["-hide_banner", "-encoders"], { encoding: "utf8" });
  if (r.error || r.status !== 0)
    throw new Error(`the video needs ffmpeg, with ${need.join(", ")} (--quick takes frames without it)`);
  const lacks = need.filter((e) => !new RegExp(`^\\s*V\\S*\\s+${e}\\s`, "m").test(r.stdout));
  if (lacks.length)
    throw new Error(
      `ffmpeg lacks ${lacks.join(", ")}, which the video needs (--quick takes frames without it)`,
    );
}

/** One theme's frames as the site's video, looping: its last 0.6 s fade into its first, which hold still. */
function encode(frames, theme) {
  const ff = (...a) => execFileSync("ffmpeg", ["-v", "error", "-y", ...a], { stdio: "inherit" });
  const fade = 0.6;
  const graph =
    `[0]split[m][head];[m]fps=${FPS},settb=1/${FPS}[main];` +
    `[head]trim=end=${fade},setpts=PTS-STARTPTS,fps=${FPS},settb=1/${FPS}[h];` +
    `[main][h]xfade=transition=fade:duration=${fade}:offset=${END - fade},scale=2200:1376:flags=lanczos,format=yuv420p[v]`;
  const input = [
    "-framerate",
    String(FPS),
    "-i",
    path.join(frames, "f%05d.png"),
    "-filter_complex",
    graph,
    "-map",
    "[v]",
  ];
  ff(
    ...input,
    "-c:v",
    "libx264",
    "-preset",
    "slow",
    "-crf",
    "17",
    "-profile:v",
    "high",
    "-level",
    "5.1",
    "-movflags",
    "+faststart",
    path.join(out, `diff-live-${theme}.mp4`),
  );
  ff(
    ...input,
    "-c:v",
    "libvpx-vp9",
    "-crf",
    "26",
    "-b:v",
    "0",
    "-row-mt",
    "1",
    "-tile-columns",
    "2",
    "-deadline",
    "good",
    "-cpu-used",
    "2",
    path.join(out, `diff-live-${theme}.webm`),
  );
  const poster = path.join(frames, `f${String(Math.round(POSTER * FPS)).padStart(5, "0")}.png`);
  ff(
    "-i",
    poster,
    "-vf",
    "scale=2200:1376:flags=lanczos",
    "-c:v",
    "libwebp",
    "-quality",
    "88",
    path.join(out, `diff-${theme}.webp`),
  );
}

const told = JSON.parse(
  execFileSync("cargo", ["xtask", "demo", "steps", TRANSCRIPT], { cwd: root, encoding: "utf8" }),
);
const { steps, whole } = story(told.steps.map((s) => ({ ...s })));
const shown = versions(told.repo, steps);
// The story ends where the transcript does, and its last step to charge_once is the transcript's.
const end = JSON.stringify(versions(told.repo, told.steps).at(-1));
if (JSON.stringify(shown.at(-1)) !== end) throw new Error("the story doesn't end where the transcript does");
if (
  !shown
    .at(-1)
    .find((f) => f.path === CHARGE)
    ?.patch.includes(whole.new.split("\n").at(-1))
) {
  throw new Error("charge_once doesn't end as the transcript leaves it");
}
if (LANDS.at(-1)[1] !== shown.length - 1)
  throw new Error(`LANDS shows ${LANDS.at(-1)[1] + 1} versions of ${shown.length}`);

if (!quick) encoders();
mkdirSync(out, { recursive: true });

const { createServer } = await import("vite");
const web = path.join(root, "web");
const vite = await createServer({
  root: web,
  configFile: path.join(web, "vite.config.ts"),
  server: { port: 5190, strictPort: false, host: "127.0.0.1" },
  logLevel: "warn",
});
// Its watchers would keep the script running after an error: it closes whatever happens.
try {
  await vite.listen();
  const url = new URL("demo/diffs.html", vite.resolvedUrls?.local[0] ?? "http://127.0.0.1:5190/").href;
  const browser = await chromium.launch();
  try {
    const demo = {
      versions: shown,
      cliRef: told.session,
      repo: told.repo,
      branch: BRANCH,
      initial: OPENS_AT,
    };
    for (const theme of themes) {
      const frames = kept
        ? path.join(kept, theme)
        : mkdtempSync(path.join(tmpdir(), `fctl-diff-video-${theme}-`));
      try {
        rmSync(frames, { recursive: true, force: true });
        mkdirSync(frames, { recursive: true });
        await record(browser, url, demo, theme, frames);
        if (quick) {
          for (const f of readdirSync(frames)) cpSync(path.join(frames, f), path.join(out, `${theme}-${f}`));
          console.log(`${theme}: frames 4 a second in ${out}`);
        } else {
          encode(frames, theme);
          console.log(`${theme}: ${out}/diff-live-${theme}.mp4, .webm and diff-${theme}.webp`);
        }
      } finally {
        // A theme's frames take over half a gigabyte: only --frames keeps them.
        if (!kept) rmSync(frames, { recursive: true, force: true });
      }
    }
  } finally {
    await browser.close();
  }
} finally {
  await vite.close();
}
