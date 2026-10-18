---
name: screenshots
description: Show how a pull request in hyperstructure/fctl changes the way fctl looks, with screenshots in its description. Covers which changes need them, before and after or only after, taking them with Playwright against `cargo xtask dev`, attaching them with `gh pr edit --attach` (never committing them), and replacing them when a later commit changes what they show. Use when a change affects what people see in the web app (a new `@hyperstructure/design` tag included), when opening its pull request, and whenever you push a commit that changes something its screenshots show.
---

# Screenshots in pull requests

A pull request that changes how fctl looks shows it in its description, so
reviewers see the change without running the branch. The screenshots show
the pull request as it is now: when a later commit changes what one shows,
replace it.

Never commit screenshots, to this or any repository. Keep them outside the
checkout, in `~/screenshots`.

## Which changes

Anything people see in the web app (`web/`): layout, styles, components,
copy, icons, empty and error states, and a new tag of
`@hyperstructure/design`. Not changes nobody sees. For a change to CLI
output, paste the output in a code block instead.

## Before and after, or after

- **Before and after** when the change alters something that exists: a
  restyle, a layout fix, new copy in place. Take both at the same size,
  theme and data, so the change is the only difference.
- **After** alone when there is nothing to compare with: a new page, dialog
  or state.

Show the part that changed, with enough around it to place it. Show both
themes unless the change looks the same in each, and a phone width (390 px)
when it affects layout. A few screenshots that show the change beat a dozen
that repeat it.

Take the before screenshots before you change anything. Afterwards, run
`main` from a worktree beside your branch, against the same control plane:

```sh
git worktree add ~/before origin/main && cd ~/before && npm ci
npm run dev --workspace web -- --port 5175   # main's web app at http://localhost:5175
```

## Taking them

Once per sandbox, install Playwright and Chromium beside the screenshots:

```sh
mkdir -p ~/screenshots && cd ~/screenshots
npm init -y >/dev/null && npm install playwright
npx playwright install --with-deps chromium   # sudo installs its system libraries
```

Run `cargo xtask dev` (in a fresh sandbox it needs `worker-build` first:
#41), and make what you want to show with the CLI and fake-claude
(`docs/development.md`): with `FCTL_SERVER` and `FCTL_TOKEN` set,
`cargo run -p fctl -- run -d -p playground "write notes.md"` starts a
session. Save this as `~/screenshots/shoot.mjs`. It signs in as the dev
owner, asks for reduced motion and seeds `Math.random`, so the WebGL scenes
hold still and draw the same frame on every load: before and after differ
only where your change does.

```js
// node shoot.mjs URL OUT.png [light|dark] [WIDTHxHEIGHT]
import { chromium } from "playwright";

const [url, out, colorScheme = "light", size = "1440x900"] = process.argv.slice(2);
const [width, height] = size.split("x").map(Number);
const browser = await chromium.launch();
const page = await browser.newPage({ colorScheme, reducedMotion: "reduce", viewport: { width, height } });
await page.addInitScript(() => {
  let seed = 42;
  Math.random = () => (seed = (seed * 16807) % 2147483647) / 2147483647;
});
await page.goto(new URL("/login", url).href);
await page.fill("input[type=email]", "dev@fctl.test");
await page.fill("input[type=password]", "fctl-dev-password");
await page.click("button[type=submit]");
await page.waitForURL((u) => u.pathname !== "/login");
await page.goto(url, { waitUntil: "networkidle" });
await page.screenshot({ path: out });
await browser.close();
```

```sh
cd ~/screenshots
node shoot.mjs http://localhost:5175/p/playground before-light.png
node shoot.mjs http://localhost:5173/p/playground after-light.png
node shoot.mjs http://localhost:5173/p/playground after-dark-390.png dark 390x844
```

For one element, use `page.locator(…).screenshot()`; for a hover, a menu or
a dialog, act on the page first. When the change is motion, record it
(`recordVideo` in `newPage`) and attach the `.webm`.

Look at every screenshot before you use it. If `npm ci` couldn't fetch
`@hyperstructure/design`, the colors and fonts are a stand-in: say so under
them. Show only the dev stack's data, never tokens, secrets or people's
details.

## Attaching them

`gh` 2.99 and newer uploads images and videos with `--attach`, as dragging
them into GitHub's editor does, and points references to those files in the
body at the uploads. Open the pull request, then add the screenshots from
`~/screenshots`:

```sh
cd ~/screenshots
gh pr view N -R hyperstructure/fctl --json body --jq .body > pr.md
# add the section below to pr.md
gh pr edit N -R hyperstructure/fctl --body-file pr.md \
  --attach before-light.png --attach after-light.png
```

Reference each file by name with Markdown image syntax (gh doesn't rewrite
`<img>` tags), and say which commit the screenshots show:

```markdown
## Screenshots

At abc1234: the project page in the light theme, 1440 px wide.

| Before | After |
|---|---|
| ![Before: the project page, light](before-light.png) | ![After: the project page, light](after-light.png) |
```

In an fctl session, `--attach` fails with "unsupported authentication type":
sessions have the GitHub App's token, and GitHub accepts uploads only with a
person's (#40). Until that changes, take the screenshots and look at them
anyway, describe what they show in the description, and say that the session
can't attach them. Don't commit them instead.

## Keeping them current

The screenshots show the pull request's newest commit. When a later commit
changes what one shows (a review fix, a tweak, `main` merged in with changes
to the same screen, a new design tag), take it again and replace it in the
same turn: nothing wakes the session afterwards.

```sh
git diff --stat abc1234 HEAD -- web/ package-lock.json   # what changed since the screenshots
cd ~/screenshots
gh pr view N -R hyperstructure/fctl --json body --jq .body > pr.md
# point the changed image at its file again, ![After: …](after-light.png), and update "At abc1234"
gh pr edit N -R hyperstructure/fctl --body-file pr.md --attach after-light.png
```

Replace the old image rather than adding the new one under it; GitHub keeps
earlier versions of the description in its edit history. Before you end a
turn that pushed commits, check that the screenshots still match.
